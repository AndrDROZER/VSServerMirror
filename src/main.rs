use clap::{Parser, Subcommand};
use core::cmp::min;
use std::fmt::format;
use fern::colors::{Color, ColoredLevelConfig};
use futures_util::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use log::{error, info, trace, warn};
use reqwest::Response;
use std::time::SystemTime;
use std::{env, io::Write};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

static APP_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"),);

#[derive(Parser, Debug)]
#[command(name = "inetm")]
#[command(author = "DROZER", about = "Command for Inet Mirroring", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Mirror vscode-servers
    #[command(arg_required_else_help = true)]
    Vserver {
        #[arg(short, long)]
        github_token: String,

        #[arg(last = true)]
        dir: Option<String>,

        #[arg(short, long, default_value_t = 5)]
        threads: usize,

        #[arg(short, long, default_value_t = 5)]
        count: usize
    },

    /// Mirror python-packages
    #[command(arg_required_else_help = true)]
    Pip { dir: Option<String> },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    setup_logger()?;

    let args = Cli::parse();
    match args.command {
        Commands::Vserver { github_token, dir, threads , count} => {
           // let token = match std::env::var("GITHUB_TOKEN") {
           //     Ok(t) => t,
           //     Err(_) => {
           //         error!("GITHUB_TOKEN variable doesn't set");
           //         return Ok(());
           //     }
           // };

            info!("Downloading vscode...!");
            process_vscode(dir, threads, github_token, count).await?;
        }

        Commands::Pip { dir: _ } => {
            info!("Downloading pip...!");
        }
    }

    Ok(())
}

fn setup_logger() -> Result<(), fern::InitError> {
    let colors = ColoredLevelConfig::new()
        // use builder methods
        .info(Color::Blue)
        .warn(Color::Yellow)
        .error(Color::Red);

    fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "[{}] {}",
                colors.color(record.level()),
                message
            ))
        })
        .level(log::LevelFilter::Info)
        .chain(std::io::stdout())
        .apply()?;
    Ok(())
}

async fn process_vscode(
    dir: Option<String>,
    threads: usize,
    token: String,
    count: usize
) -> Result<(), Box<dyn std::error::Error>> {
    let path = dir.unwrap_or("vscode".to_owned());
    match std::fs::create_dir(&path) {
        Err(_err) => {}
        _ => {}
    };

    let arch = "x64".to_owned();
    let owner = "microsoft".to_owned();
    let repo = "vscode".to_owned();

    // let commits = get_releases(owner, repo).await?;

    let release_sha_vec = get_releases(owner, repo, &threads, token, count).await?;
    let mut set = tokio::task::JoinSet::new();

    let multiple_bar = MultiProgress::new();
    multiple_bar.set_draw_target(ProgressDrawTarget::stdout_with_hz(30));
    for release in release_sha_vec {
        let download_url = format!(
            "https://update.code.visualstudio.com/commit:{}/server-linux-{}/stable",
            release, &arch
        );
        let task = download_server(
            download_url,
            arch.clone(),
            path.clone(),
            release.clone(),
            multiple_bar.clone(),
        );
        if set.len() == threads {
            let _ = match set.join_next().await {
                Some(r) => r,
                None => Ok(()),
            };
            set.spawn(task);
        } else {
            set.spawn(task);
        }
    }

    while let Some(res) = set.join_next().await {
        match res {
            Err(err) => {
                error!("{}", err);
            }
            _ => {}
        };
    }

    multiple_bar.clear().unwrap();
    Ok(())
}

async fn download_server(
    download_url: String,
    arch: String,
    path: String,
    release: String,
    multiple_bar: MultiProgress,
) {
    let client = reqwest::Client::new();
    let response: Response = match client.get(&download_url).send().await {
        Err(err) => {
            error!("{}. URL: {}", err, download_url);
            return;
        }
        Ok(resp) => resp,
    };

    let mut dest: File = {
        let fname = format!("vscode-server-linux-{}.tar.gz", arch);

        match tokio::fs::metadata(format!("{}/{}", path, release)).await {
            Ok(_) => {},
            Err(_) => {
                match std::fs::create_dir(format!("{}/{}", path, release)) {
                    Err(err) => {
                        error!("{}", err);
                    }
                    _ => {}
                };        
            },
        }

        match tokio::fs::metadata(format!("{}/{}/{}", path, release, fname)).await {
            Ok(_) => {
                info!("{} already downloaded, skipping!", release);
                return;
            },
            Err(_) => {},
        }

        match tokio::fs::File::create(format!("{}/{}/{}", path, release, fname)).await {
            Err(err) => {
                error!("{}. URL: {}", err, download_url);
                return;
            }
            Ok(dest) => dest,
        }
    };

    let total_size = response.content_length().unwrap_or(0);

    let pb = multiple_bar.add(ProgressBar::new(total_size / 1024));
    let style = ProgressStyle::with_template(
        "[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}",
    )
    .unwrap()
    .progress_chars("#>-");
    pb.set_message(format!("KB. {}", release));
    pb.set_style(style);

    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    while let Some(item) = stream.next().await {
        let chunk = match item {
            Ok(ch) => ch,
            Err(err) => {
                error!("{}", err);
                return;
            }
        };
        let _ = dest.write_all(&chunk).await;
        let _ = dest.flush().await;
        let new = min(downloaded + (chunk.len() as u64) / 1024, total_size);
        downloaded = new;
        pb.set_position(new);
    }

    pb.finish_with_message(format!("Release {} downloaded", release));
}

async fn get_releases(
    owner: String,
    repo: String,
    threads: &usize,
    token: String,
    count: usize
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let github_repo_info_url = format!("https://api.github.com/repos/{}/{}/releases?per_page={}", owner, repo, count);
    let tag_arr = collect_tags_from_github_repo(&github_repo_info_url, token.clone()).await?;

    let sha_arr: Vec<String> =
        collect_sha_from_github_repo(owner, repo, &tag_arr, threads, token).await?;

    Ok(sha_arr)
}

async fn collect_tags_from_github_repo(
    url: &String,
    token: String,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let client = reqwest::Client::builder()
        .user_agent(APP_USER_AGENT)
        .build()?;
    let res = client
        .get(url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?
        .text()
        .await?;

    let raw_data: serde_json::Value = serde_json::from_str(&res)?;

    let mut counter = 0;
    let mut tags: Vec<String> = Vec::new();

    while raw_data[counter] != serde_json::Value::Null {
        tags.push(format!("{}", raw_data[counter]["tag_name"]).replace("\"", ""));
        counter += 1;
    }

    //println!("{}", tags[0]);

    Ok(tags)
}

async fn collect_sha_from_github_repo(
    owner: String,
    repo: String,
    tag_arr: &Vec<String>,
    threads: &usize,
    token: String,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let tag_info_url = format!(
        "https://api.github.com/repos/{}/{}/git/ref/tags",
        owner, repo
    );

    let mut sha_vec: Vec<String> = Vec::new();
    let mut set = tokio::task::JoinSet::new();

    for tag in tag_arr {
        let tag_url = format!("{}/{}", tag_info_url, tag);
        //println!("{}", tag_url);
        let task = get_sha(tag_url, owner.clone(), repo.clone(), token.clone());
        if set.len() == threads.to_owned() {
            while let Some(res) = set.join_next().await {
                match res {
                    Err(err) => {
                        error!("{}", err);
                    }
                    Ok(e) => match e {
                        Some(s) => sha_vec.push(s),
                        _ => {}
                    },
                };
            }
            set.spawn(task);
        } else {
            set.spawn(task);
        }
    }

    while let Some(res) = set.join_next().await {
        match res {
            Err(err) => {
                error!("{}", err);
            }
            Ok(e) => match e {
                Some(s) => sha_vec.push(s),
                _ => {}
            },
        }
    }

    info!("SHA hashes collected, total: {}", sha_vec.len());
    //println!("{}", sha_vec[0]);
    Ok(sha_vec)
}

async fn get_sha(tag_url: String, owner: String, repo: String, token: String) -> Option<String> {
    let client: reqwest::Client = match reqwest::Client::builder()
        .user_agent(APP_USER_AGENT)
        .build()
    {
        Err(err) => {
            error!("{}", err);
            return None;
        }
        Ok(c) => c,
    };

    let resp: reqwest::Response = match client
        .get(tag_url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
    {
        Err(err) => {
            error!("{}", err);
            return None;
        }
        Ok(c) => c,
    };

    let res = match resp.text().await {
        Err(err) => {
            error!("{}", err);
            return None;
        }
        Ok(c) => c,
    };

    let raw_data: serde_json::Value = match serde_json::from_str(&res) {
        Err(err) => {
            error!("{}", err);
            return None;
        }
        Ok(c) => c,
    };

    if raw_data["object"]["type"] == "commit" {
        Some(format!("{}", raw_data["object"]["sha"]).replace("\"", ""))
    } else {
        let url = format!(
            "https://api.github.com/repos/{}/{}/git/tags/{}",
            owner, repo, raw_data["object"]["sha"]
        )
        .replace("\"", "");
        let resp = match client
            .get(url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
        {
            Err(err) => {
                error!("{}", err);
                return None;
            }
            Ok(c) => c,
        };

        let res = match resp.text().await {
            Err(err) => {
                error!("{}", err);
                return None;
            }
            Ok(c) => c,
        };

        let raw_data: serde_json::Value = match serde_json::from_str(&res) {
            Err(err) => {
                error!("{}", err);
                return None;
            }
            Ok(c) => c,
        };

        Some(format!("{}", raw_data["object"]["sha"]).replace("\"", ""))
    }
}
