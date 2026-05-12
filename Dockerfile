# ============================================================
# ContractGate — Rust backend Dockerfile
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

# Copy dependency manifests first to cache the crate fetch layer
COPY Cargo.toml Cargo.lock ./

# Build a dummy main to pre-fetch and compile all dependencies
RUN mkdir -p src && echo "fn main() {}" > src/main.rs
RUN cargo build --release 2>&1 || true
# Remove the dummy artifact so the real build picks up src/
RUN rm -f target/release/contractgate* target/release/deps/contractgate*

# Now copy real source and build.
# demo/ must be present alongside src/ because stream_demo.rs embeds the
# scenario YAML files at compile time via include_str!("../demo/scenarios/…").
# contracts/ must be present because demo-seeder embeds starter YAMLs via
# include_str!("../../contracts/starters/…") (RFC-017).
COPY demo ./demo
COPY contracts ./contracts
COPY src ./src
#RUN cargo build --release
# Build the web server and demo-seeder binaries (explicit names)
RUN cargo build --release --bin contractgate-server --features kafka-ingress,scaffold,kinesis-ingress
RUN cargo build --release --bin demo-seeder

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

# Fly.io / Railway will inject PORT automatically
ENV PORT=3001
EXPOSE 3001

HEALTHCHECK --interval=15s --timeout=5s --start-period=10s --retries=3 \
    CMD wget -qO- http://localhost:${PORT}/health || exit 1

ENTRYPOINT ["contractgate"]
