FROM rust:1-bookworm

WORKDIR /workspace

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY tests ./tests
COPY packs ./packs
COPY web ./web
COPY nomad ./nomad

RUN cargo test --no-run

EXPOSE 3000

ENV APP_BIND=0.0.0.0:3000

CMD ["cargo", "run"]
