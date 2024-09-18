# Description

This project developed for creating self-hosted vscode server. This can be useful when you need to work on a remote host from a computer that does not have access to the Internet (for example, in a closed network segment).

# Build

This is rust, so it's simple (tested only for x64)

```bash
cargo build
```

# Run

```bash
vserver -g <YOUR_GITHUB_TOKEN>
```

All options can be viewed by `--help` flag.

# Configuration

If you need to host your own web server to distribute packages, you can use the `nginx.conf` file with your nginx installation.

# Docker

You can build and run app image:
```
docker build . -t vsmirror
docker run -v ./vscode:/usr/src/vsmirror/vscode -it --rm vsmirror vserver -g <YOUR_GITHUB_TOKEN>
```

```
docker run --name vsmirror_nginx \
    -v $pwd/nginx.conf:/etc/nginx/nginx.conf.d:ro -p 8080:80 \ 
    -v vscode:/mnt/repos/vscode \ 
    -v <YOUR_CERT>:/etc/nginx/certs/cert.crt \
    -v <YOUR_KEY>:/etc/nginx/certs/key.key \
    -d nginx
```