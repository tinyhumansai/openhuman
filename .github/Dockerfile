FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive

# System deps for Tauri + mold linker + clang (for mold)
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    cmake \
    curl \
    ca-certificates \
    git \
    pkg-config \
    libgtk-3-dev \
    libwebkit2gtk-4.1-dev \
    libappindicator3-dev \
    librsvg2-dev \
    patchelf \
    mold \
    clang \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Rust 1.93.0 with minimal profile + fmt/clippy
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH="/usr/local/cargo/bin:$PATH"
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- \
    -y --default-toolchain 1.93.0 --profile minimal \
    -c rustfmt -c clippy \
    -t x86_64-unknown-linux-gnu

# Node.js 24.x + yarn
RUN curl -fsSL https://deb.nodesource.com/setup_24.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && npm install -g yarn

# Install system dependencies (cmake, ALSA, X11)
RUN apt-get update && apt-get install -y --no-install-recommends cmake libasound2-dev libxdo-dev libxtst-dev libx11-dev libevdev-dev webkit2gtk-driver && rm -rf /var/lib/apt/lists/*

# sccache (pre-installed so the action only needs to configure it)
RUN curl -fsSL https://github.com/mozilla/sccache/releases/download/v0.10.0/sccache-v0.10.0-x86_64-unknown-linux-musl.tar.gz \
    | tar xz -C /usr/local/bin --strip-components=1 sccache-v0.10.0-x86_64-unknown-linux-musl/sccache \
    && chmod +x /usr/local/bin/sccache

# Verify installs
RUN rustc --version && cargo --version && node --version && yarn --version && mold --version && sccache --version
