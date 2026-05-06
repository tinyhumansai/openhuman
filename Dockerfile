# ---------------------------------------------------------------------------
# OpenHuman Core — multi-stage Docker build
# Produces a minimal image running the `openhuman-core` binary (JSON-RPC server).
#
# Build:   docker build -t openhuman-core .
# Run:     docker run -p 7788:7788 --env-file .env openhuman-core
# ---------------------------------------------------------------------------

# ==========================================================================
# Stage 1: Build the Rust binary
# ==========================================================================
FROM rust:1.93-bookworm AS builder

ENV DEBIAN_FRONTEND=noninteractive

# System dependencies required for compilation.
#
# ALSA / X11 / input headers are needed because `cpal`, `enigo`, `arboard`,
# and `rdev` are unconditional dependencies of the core crate (used by the
# voice, autocomplete, and clipboard subsystems). They link against system
# libraries even when the corresponding features are disabled at runtime.
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    cmake \
    pkg-config \
    libssl-dev \
    libasound2-dev \
    libxdo-dev \
    libxtst-dev \
    libx11-dev \
    libevdev-dev \
    clang \
    mold \
    ca-certificates \
    git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Cache dependencies — copy only manifests first
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
# Create a dummy src to build deps
RUN mkdir -p src && \
    echo 'fn main() {}' > src/main.rs && \
    echo 'pub fn run_core_from_args(_: &[String]) -> anyhow::Result<()> { Ok(()) }' > src/lib.rs && \
    cargo build --release --bin openhuman-core 2>/dev/null || true && \
    rm -rf src

# Copy actual source and build
COPY src/ src/
# Touch main.rs to force rebuild of our code (not deps)
RUN touch src/main.rs src/lib.rs && \
    cargo build --release --bin openhuman-core

# ==========================================================================
# Stage 2: Minimal runtime image
# ==========================================================================
FROM debian:bookworm-slim AS runtime

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    libasound2 \
    libxdo3 \
    libxtst6 \
    libx11-6 \
    libevdev2 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Non-root user for security
RUN useradd --create-home --shell /bin/bash openhuman
USER openhuman
WORKDIR /home/openhuman

# Copy the built binary
COPY --from=builder /build/target/release/openhuman-core /usr/local/bin/openhuman-core

# Default workspace directory
ENV OPENHUMAN_WORKSPACE=/home/openhuman/.openhuman
# Bind to all interfaces so the container is reachable
ENV OPENHUMAN_CORE_HOST=0.0.0.0
ENV OPENHUMAN_CORE_PORT=7788
ENV RUST_LOG=info

EXPOSE 7788

# Health check against the root endpoint
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -sf http://localhost:7788/health || exit 1

ENTRYPOINT ["openhuman-core"]
CMD ["serve"]
