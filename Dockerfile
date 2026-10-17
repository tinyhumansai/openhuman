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
# Keep in step with rust-toolchain.toml; an older image makes rustup download the
# pinned toolchain on every uncached build.
FROM rust:1.96.1-bookworm AS builder

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
COPY Cargo.toml Cargo.lock rust-toolchain.toml build.rs README.md ./
COPY crates/openhuman-cli/Cargo.toml crates/openhuman-cli/Cargo.toml
COPY crates/openhuman-core/Cargo.toml crates/openhuman-core/Cargo.toml
COPY crates/openhuman-embed/Cargo.toml crates/openhuman-embed/Cargo.toml
COPY crates/openhuman-modules-linked/Cargo.toml crates/openhuman-modules-linked/Cargo.toml
COPY crates/openhuman-rpc/Cargo.toml crates/openhuman-rpc/Cargo.toml
COPY crates/openhuman-tinyhumans/Cargo.toml crates/openhuman-tinyhumans/Cargo.toml
COPY crates/openhuman-tui/Cargo.toml crates/openhuman-tui/Cargo.toml
# Vendored TinyAgents SDK (git submodule; [patch.crates-io] points here, so
# the dep-cache build below already resolves it). CI must init the submodule
# before docker build — see the "Init tinyagents submodule" steps in
# release-production.yml / release-staging.yml.
COPY vendor/ vendor/
# Create a dummy src to build deps. openhuman-modules-linked and openhuman-tui
# keep theirs: the image never builds them, but cargo must load every member.
RUN mkdir -p crates/openhuman-cli/src crates/openhuman-core/src crates/openhuman-embed/src \
             crates/openhuman-modules-linked/src crates/openhuman-rpc/src \
             crates/openhuman-tinyhumans/src crates/openhuman-tui/src && \
    echo 'fn main() {}' > crates/openhuman-cli/src/main.rs && \
    echo 'pub fn run_core_from_args(_: &[String]) -> anyhow::Result<()> { Ok(()) }' > crates/openhuman-core/src/lib.rs && \
    echo '' > crates/openhuman-embed/src/lib.rs && \
    echo '' > crates/openhuman-modules-linked/src/lib.rs && \
    echo '' > crates/openhuman-rpc/src/lib.rs && \
    echo '' > crates/openhuman-tinyhumans/src/lib.rs && \
    echo 'fn main() {}' > crates/openhuman-tui/src/main.rs && \
    echo 'pub fn run_from_cli(_: &[String]) -> anyhow::Result<()> { Ok(()) }' > crates/openhuman-tui/src/lib.rs && \
    cargo build --profile "${CARGO_PROFILE}" -p openhuman-cli --bin openhuman-core 2>/dev/null || true && \
    rm -rf crates/openhuman-cli/src crates/openhuman-core/src crates/openhuman-embed/src \
           crates/openhuman-rpc/src crates/openhuman-tinyhumans/src

# Copy actual source and build
COPY crates/openhuman-cli/src/ crates/openhuman-cli/src/
COPY crates/openhuman-core/src/ crates/openhuman-core/src/
COPY crates/openhuman-embed/src/ crates/openhuman-embed/src/
COPY crates/openhuman-rpc/src/ crates/openhuman-rpc/src/
COPY crates/openhuman-tinyhumans/src/ crates/openhuman-tinyhumans/src/
# Touch every crate the dep-cache stage built from a dummy src. COPY keeps the
# checkout's mtimes, which predate that build, so cargo would otherwise link
# core against the empty openhuman-rpc placeholder.
RUN touch crates/openhuman-cli/src/main.rs crates/openhuman-core/src/lib.rs \
          crates/openhuman-embed/src/lib.rs crates/openhuman-rpc/src/lib.rs \
          crates/openhuman-tinyhumans/src/lib.rs && \
    cargo build --profile "${CARGO_PROFILE}" -p openhuman-cli --bin openhuman-core && \
    cp "target/${CARGO_PROFILE}/openhuman-core" /tmp/openhuman-core

# ==========================================================================
# Stage 2: Stage the registry-pinned native modules at build time
# ==========================================================================
# Downloads and digest-verifies every module archive the compiled registry
# pins for this image's architecture (buildx runs this stage per platform), so
# the container never needs GitHub access to load a module. The oldest
# published Linux build (ubuntu-22.04, glibc 2.35) runs on bookworm (2.36).
FROM node:24-bookworm-slim AS modules
WORKDIR /repo
COPY scripts/lib/module-pins.mjs scripts/lib/module-pins.mjs
COPY scripts/ci/self-hosted/test-module-assets.mjs scripts/ci/self-hosted/test-module-assets.mjs
COPY scripts/release/stage-modules.mjs scripts/release/stage-modules.mjs
COPY crates/openhuman-core/src/modules/registry.rs crates/openhuman-core/src/modules/registry.rs
COPY crates/openhuman-core/src/modules/registry/ crates/openhuman-core/src/modules/registry/
RUN node scripts/release/stage-modules.mjs --output /bundled-modules

# ==========================================================================
# Stage 3: Minimal runtime image
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
# Create every directory a named volume is mounted over, so a fresh volume
# inherits this ownership instead of being root-owned (Docker only copies
# ownership from the image when the mount point already exists there).
# `~/OpenHuman` is the agent's default projects/action directory, mounted by
# docker-compose.yml as `openhuman-projects`.
RUN mkdir -p /home/openhuman/.openhuman /home/openhuman/OpenHuman \
 && chown -R openhuman:openhuman /home/openhuman

# Copy the built binary
COPY --from=builder /tmp/openhuman-core /usr/local/bin/openhuman-core

# Pinned native modules, verified at image build time. The core still checks
# each archive against the compiled digest and tinybus admission before loading.
COPY --from=modules /bundled-modules /opt/openhuman/bundled-modules
ENV OPENHUMAN_BUNDLED_MODULES=/opt/openhuman/bundled-modules

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
#
# CAUTION: because the image default user is root, `docker exec <ctr> ...` lands
# as root and does NOT run the entrypoint — so running `openhuman-core` that way
# creates a root-owned `config.toml` (the core writes it at mode 0600), which
# uid 10001 then cannot read on the next start. Use
# `docker exec -u openhuman <ctr> openhuman-core ...` for any CLI poking around.
# The entrypoint heals a workspace already in that state, but prevention is
# cheaper than a restart loop.
USER root

# Default workspace directory
ENV OPENHUMAN_WORKSPACE=/home/openhuman/.openhuman
# Bind to all interfaces so the container is reachable
ENV OPENHUMAN_CORE_HOST=0.0.0.0
ENV OPENHUMAN_CORE_PORT=7788
# Stable first-party signal for CLI launch policy; containers default headless.
ENV OPENHUMAN_DOCKER=1
ENV RUST_LOG=info
# AgentBox marketplace mode — off by default for desktop builds. The
# AgentBox console flips this on per deployment, along with GMI_MAAS_*.
ENV OPENHUMAN_AGENTBOX_MODE=0

EXPOSE 7788

# Health check against the root endpoint
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -sf http://localhost:7788/health || exit 1

ENTRYPOINT ["/usr/local/bin/docker-entrypoint-core.sh"]
CMD ["serve"]
