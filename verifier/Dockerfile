FROM mcr.microsoft.com/playwright:v1.55.1-noble

ARG RUST_TOOLCHAIN=1.88.0
ENV RUSTUP_HOME=/opt/rust/rustup \
    CARGO_HOME=/opt/rust/cargo \
    RUSTUP_TOOLCHAIN=${RUST_TOOLCHAIN} \
    PATH=/opt/rust/cargo/bin:/opt/practice-studio/node_modules/.bin:$PATH

RUN apt-get update \
    && apt-get install -y --no-install-recommends curl ca-certificates build-essential pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/* \
    && curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs \
       | sh -s -- -y --profile minimal --default-toolchain ${RUST_TOOLCHAIN}

WORKDIR /opt/practice-studio
COPY verifier/package.json verifier/package-lock.json ./
RUN npm ci --ignore-scripts && npx playwright install chromium

COPY verifier/rust-cache ./rust-cache
RUN cargo generate-lockfile --manifest-path rust-cache/Cargo.toml \
    && cargo fetch --locked --manifest-path rust-cache/Cargo.toml

COPY verifier/verify.mjs ./verify.mjs
RUN printf '#!/bin/sh\nexec node /opt/practice-studio/verify.mjs "$@"\n' > /opt/practice-studio/verify \
    && chmod 0555 /opt/practice-studio/verify \
    && chmod -R a=rX /opt/practice-studio /opt/rust

USER 65532:65532
CMD ["/opt/practice-studio/verify"]
