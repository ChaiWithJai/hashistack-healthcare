# syntax=docker/dockerfile:1.7
FROM rust:1.86-bookworm@sha256:300ec56abce8cc9448ddea2172747d048ed902a3090e6b57babb2bf19f754081 AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY export-assets ./export-assets
COPY packs ./packs
COPY web ./web
COPY nomad ./nomad
COPY vault ./vault
COPY staging/identities.hcl ./staging/identities.hcl
COPY migrations ./migrations
RUN --mount=type=cache,id=studio-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=studio-cargo-target,target=/src/target \
    cargo build --locked --release \
    && install -Dm0755 /src/target/release/rust-proof-service /out/studio

FROM debian:bookworm-slim@sha256:60eac759739651111db372c07be67863818726f754804b8707c90979bda511df AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 65532 studio \
    && useradd --system --uid 65532 --gid 65532 --no-create-home studio \
    && install -d -o 65532 -g 65532 -m 0700 /var/lib/studio
COPY --from=builder /out/studio /usr/local/bin/studio
USER 65532:65532
EXPOSE 3000
ENV APP_BIND=0.0.0.0:3000
ENTRYPOINT ["/usr/local/bin/studio"]
