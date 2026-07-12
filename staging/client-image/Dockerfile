FROM rust:1-bookworm AS build

WORKDIR /build
COPY packs/hypertension-tracker/scaffold/Cargo.toml ./Cargo.toml
COPY packs/hypertension-tracker/scaffold/Cargo.lock ./Cargo.lock
COPY packs/hypertension-tracker/scaffold/src ./src
COPY packs/hypertension-tracker/synthetic /synthetic
RUN cargo build --release

FROM debian:bookworm-slim
RUN useradd --uid 65532 --no-create-home --shell /usr/sbin/nologin app
COPY --from=build /build/target/release/app /usr/local/bin/app
USER 65532:65532
EXPOSE 8080
ENV APP_BIND=0.0.0.0:8080
ENTRYPOINT ["/usr/local/bin/app"]
