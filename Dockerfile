# syntax=docker/dockerfile:1.7
FROM rust:1.85-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY packs ./packs
COPY web ./web
COPY nomad ./nomad
COPY vault ./vault
COPY staging/identities.hcl ./staging/identities.hcl
COPY migrations ./migrations
RUN cargo build --locked --release

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 65532 studio \
    && useradd --system --uid 65532 --gid 65532 --no-create-home studio \
    && install -d -o 65532 -g 65532 -m 0700 /var/lib/studio
COPY --from=builder /src/target/release/rust-proof-service /usr/local/bin/studio
USER 65532:65532
EXPOSE 3000
ENV APP_BIND=0.0.0.0:3000
ENTRYPOINT ["/usr/local/bin/studio"]
