FROM lukemathwalker/cargo-chef:latest-rust-bookworm AS chef
WORKDIR /app

# Build deps required by workspace crates:
# cmake            → aws-lc-sys
# clang/libclang   → librocksdb-sys (bindgen)
# protobuf-compiler → blueprint-manager-bridge (prost)
RUN apt-get update && \
    apt-get install -y --no-install-recommends cmake clang libclang-dev protobuf-compiler libprotobuf-dev && \
    rm -rf /var/lib/apt/lists/*

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

FROM ghcr.io/foundry-rs/foundry:latest AS foundry

FROM chef AS app-builder
COPY . .

COPY --from=foundry /usr/local/bin/forge /usr/local/bin/forge
COPY --from=foundry /usr/local/bin/cast /usr/local/bin/cast
COPY --from=foundry /usr/local/bin/anvil /usr/local/bin/anvil

RUN cargo build --release -p ai-agent-sandbox-blueprint-bin

FROM debian:bookworm-slim AS runtime
WORKDIR /app

RUN apt-get update && \
    apt-get install -y --no-install-recommends libssl3 curl && \
    rm -rf /var/lib/apt/lists/* && \
    (groupadd -r operator 2>/dev/null || true) && \
    (useradd -r -g operator -d /app -s /sbin/nologin operator 2>/dev/null || true) && \
    mkdir -p /app/keystore /app/blueprint-state && \
    chown -R operator:operator /app

COPY --from=app-builder /app/target/release/ai-agent-sandbox-blueprint /usr/local/bin/

LABEL org.opencontainers.image.authors="drewstone <drewstone329@gmail.com>"
LABEL org.opencontainers.image.description="AI Agent Sandbox Blueprint"
LABEL org.opencontainers.image.source="https://github.com/tangle-network/ai-agent-sandbox-blueprint"
LABEL org.opencontainers.image.licenses="MIT OR Apache-2.0"

ENV RUST_LOG="info"
ENV BIND_ADDR="0.0.0.0"
ENV BLUEPRINT_ID=0
ENV SERVICE_ID=0
ENV CHAIN="testnet"
ENV KEYSTORE_URI="/app/keystore"
ENV DATA_DIR="/app/blueprint-state"
ENV OPERATOR_API_PORT=8080
ENV ALLOW_STANDALONE=true

HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=5 \
    CMD curl -sf http://localhost:${OPERATOR_API_PORT}/health || exit 1

# The entrypoint uses --data-dir from DATA_DIR env. All other config
# (RPC URLs, keys, ports, secrets) comes from env vars set by the operator
# via sandbox-runtime's build_env_vars().
ENTRYPOINT ["/usr/local/bin/ai-agent-sandbox-blueprint", "run", \
    "--protocol", "tangle", "-t", \
    "--data-dir", "/app/blueprint-state"]
