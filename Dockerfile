FROM rust:1-bookworm

WORKDIR /workspace

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY tests ./tests

RUN cargo test --no-run

EXPOSE 3000

CMD ["cargo", "run"]
