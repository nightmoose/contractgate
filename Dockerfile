# ============================================================
# ContractGate — Rust backend Dockerfile
# Multi-stage build: builder → slim runtime image
# Target: <20 MB final image, optimised release binary
# ============================================================

# ── Stage 1: build ──────────────────────────────────────────
FROM rust:1.86-slim-bookworm AS builder

WORKDIR /app

# Install native TLS dependencies needed by sqlx
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy dependency manifests first to cache the crate fetch layer
COPY Cargo.toml Cargo.lock ./

# Build a dummy main to pre-fetch and compile all dependencies
RUN mkdir -p src && echo "fn main() {}" > src/main.rs
RUN cargo build --release 2>&1 || true
# Remove the dummy artifact so the real build picks up src/
RUN rm -f target/release/contractgate* target/release/deps/contractgate*

# Now copy real source and build
COPY src ./src
RUN cargo build --release

# ── Stage 2: runtime ────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

WORKDIR /app

# Install CA certificates and libssl for TLS (required by sqlx / native-tls)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy only the compiled binary
COPY --from=builder /app/target/release/contractgate /usr/local/bin/contractgate

# Non-root user for security
RUN adduser --disabled-password --gecos "" --uid 1001 appuser
USER appuser

# Fly.io / Railway will inject PORT automatically
ENV PORT=3001
EXPOSE 3001

HEALTHCHECK --interval=15s --timeout=5s --start-period=10s --retries=3 \
    CMD wget -qO- http://localhost:${PORT}/health || exit 1

ENTRYPOINT ["contractgate"]
