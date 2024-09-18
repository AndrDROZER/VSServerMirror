FROM rust:1.75

WORKDIR /usr/src/vsmirror
COPY ./src ./src
COPY Cargo.toml .

RUN cargo install --path .

ENTRYPOINT ["vsmirror"]
