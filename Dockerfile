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

# Docker builds often run on small VPS/CI builders. The crate's `ci` profile
# keeps peak rustc memory lower than `release`; override with
# `--build-arg CARGO_PROFILE=release` when maximum runtime optimization matters.
ARG CARGO_PROFILE=ci
ARG CARGO_BUILD_JOBS=1
ENV DEBIAN_FRONTEND=noninteractive \
    CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}

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
    cargo build --profile "${CARGO_PROFILE}" --bin openhuman-core 2>/dev/null || true && \
    rm -rf src

# Copy actual source and build
COPY src/ src/
# Touch main.rs to force rebuild of our code (not deps)
RUN touch src/main.rs src/lib.rs && \
    cargo build --profile "${CARGO_PROFILE}" --bin openhuman-core && \
    cp "target/${CARGO_PROFILE}/openhuman-core" /tmp/openhuman-core

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
    gosu \
    && rm -rf /var/lib/apt/lists/*

# Non-root user for security — fixed UID/GID so volume ownership is stable
# across image rebuilds.
RUN groupadd --gid 10001 openhuman \
 && useradd --uid 10001 --gid 10001 --create-home --shell /bin/bash openhuman

# Pre-create and own the workspace directory inside the image so the
# entrypoint chown is a no-op on a fresh (root-owned) named volume and on
# first-time anonymous volume mounts.
ENV HOME=/home/openhuman
RUN mkdir -p /home/openhuman/.openhuman \
 && chown -R openhuman:openhuman /home/openhuman

# Copy the built binary
COPY --from=builder /tmp/openhuman-core /usr/local/bin/openhuman-core

# Copy the entrypoint script that chowns the workspace volume before dropping
# privileges.  The script is a separate file so the E2E entrypoint
# (e2e/docker-entrypoint.sh) is not affected.
COPY scripts/docker-entrypoint-core.sh /usr/local/bin/docker-entrypoint-core.sh
# Windows checkouts may materialize shell scripts with CRLF line endings when
# core.autocrlf is enabled.  A CRLF shebang makes Linux report the executable
# as "no such file or directory" at container startup, so normalize in-image.
RUN sed -i 's/\r$//' /usr/local/bin/docker-entrypoint-core.sh \
 && chmod +x /usr/local/bin/docker-entrypoint-core.sh

# The entrypoint runs as root so it can chown the mounted volume, then execs
# gosu to drop to the openhuman user before starting the binary.
USER root

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

ENTRYPOINT ["/usr/local/bin/docker-entrypoint-core.sh"]
CMD ["serve"]
