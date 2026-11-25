# Build from the repository root:
#   docker build -t dcg-review -f examples/readiness-review/Dockerfile .
#   docker run --rm -v "$PWD/examples/readiness-review:/data" dcg-review /data/orders.csv --out /data/out
FROM rust:1.98-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY src ./src
COPY ops ./ops
# The bench binary is referenced by Cargo.toml; copy just enough to build the CLI.
COPY . .
RUN cargo build --release --bin contractgate

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/contractgate /usr/local/bin/contractgate
WORKDIR /data
ENTRYPOINT ["contractgate", "review"]
