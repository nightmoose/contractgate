# ============================================================
# DataContractGate — Rust backend Dockerfile
# Multi-stage build: builder → slim runtime image
# Target: <20 MB final image, optimised release binary
# ============================================================

# ── Stage 1: build ──────────────────────────────────────────
FROM rust:slim-bookworm AS builder

WORKDIR /app

# sqlx + reqwest both use runtime-tokio-rustls / rustls-tls (pure Rust TLS).
# No OpenSSL headers needed — libssl-dev is intentionally absent.
# pkg-config kept for any transitive crate that probes for system libs at
# build time; it's a no-op if nothing uses it.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    libssl3 \
    libssl-dev \
    libsasl2-dev \
    libcurl4-openssl-dev \
    zlib1g-dev \
    libzstd-dev \
    liblz4-dev \
    libsnappy-dev \
    cmake \
    build-essential \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Copy dependency manifests first so the dependency layer below is cached
# until Cargo.toml / Cargo.lock change.
COPY Cargo.toml Cargo.lock ./

# Compile every dependency once, against stub sources. Every target declared
# in Cargo.toml needs a stub or cargo refuses the manifest, and the build
# must use the same bins + features as the real build below or its deps
# don't match. (Until 2026-10-13 this stubbed only main.rs, failed in 2s
# behind `|| true`, and every image recompiled all ~360 crates.)
ARG CARGO_BUILD_ARGS="--release --bin contractgate-server --bin demo-seeder --features kafka-ingress,scaffold,kinesis-ingress"
RUN mkdir -p src/bin \
    && touch src/lib.rs \
    && for f in src/main.rs src/bin/contractgate.rs src/bin/demo.rs src/bin/demo-seeder.rs; do \
         echo "fn main() {}" > "$f"; \
       done \
    && cargo build $CARGO_BUILD_ARGS \
    && rm -rf src \
    && rm -rf target/release/.fingerprint/contractgate-* target/release/.fingerprint/demo-seeder-* \
              target/release/deps/contractgate* target/release/deps/libcontractgate* \
              target/release/deps/demo_seeder* \
              target/release/contractgate-server target/release/demo-seeder

# Now copy real source and build.
# demo/ must be present alongside src/ because stream_demo.rs embeds the
# scenario YAML files at compile time via include_str!("../demo/scenarios/…").
# contracts/ must be present because demo-seeder embeds starter YAMLs via
# include_str!("../../contracts/starters/…") (RFC-017).
COPY demo ./demo
COPY contracts ./contracts
COPY src ./src
# Build the web server and demo-seeder binaries in one invocation with the
# same args as the dependency layer, so only this crate compiles here.
RUN cargo build $CARGO_BUILD_ARGS

# ── Stage 2: runtime ────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

WORKDIR /app

# Install CA certificates, libssl, and curl (used by Compose healthcheck)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy compiled binaries
#COPY --from=builder /app/target/release/contractgate /usr/local/bin/contractgate
COPY --from=builder /app/target/release/contractgate-server /usr/local/bin/contractgate
COPY --from=builder /app/target/release/demo-seeder /usr/local/bin/demo-seeder

# Non-root user for security
RUN adduser --disabled-password --gecos "" --uid 1001 appuser
USER appuser

# Commit this image was built from; /health reports it as "build" so a
# deploy can be confirmed live. CI passes --build-arg GIT_SHA=<sha>.
ARG GIT_SHA=unknown
ENV GIT_SHA=${GIT_SHA}

# Fly.io / Railway will inject PORT automatically
ENV PORT=3001
EXPOSE 3001

HEALTHCHECK --interval=15s --timeout=5s --start-period=10s --retries=3 \
    CMD wget -qO- http://localhost:${PORT}/health || exit 1

ENTRYPOINT ["contractgate"]
