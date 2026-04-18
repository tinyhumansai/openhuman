FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive

# System deps for Tauri + mold linker + clang + E2E testing (xvfb, dbus, webkit2gtk-driver).
# CEF (bundled Chromium) runtime libs: libnss3, libnspr4, libgbm1, libxshmfence1,
# libxkbcommon0, libatk-bridge2.0-0 — required to link/run the CEF shared lib that
# cef-dll-sys downloads at build time (CEF is the default runtime now).
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
    libclang-dev \
    libssl-dev \
    xvfb \
    at-spi2-core \
    dbus-x11 \
    webkit2gtk-driver \
    libnss3 \
    libnspr4 \
    libgbm1 \
    libxshmfence1 \
    libxkbcommon0 \
    libatk-bridge2.0-0 \
    libxcomposite1 \
    libxdamage1 \
    libxrandr2 \
    libcups2 \
    libpangocairo-1.0-0 \
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

# Install ALSA, X11, input, and E2E automation dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    libasound2-dev libxdo-dev libxtst-dev libx11-dev libevdev-dev \
    webkit2gtk-driver \
    && rm -rf /var/lib/apt/lists/*

# Install system dependencies (cmake, ALSA, X11)
RUN apt-get update && apt-get install -y --no-install-recommends cmake libasound2-dev libxdo-dev libxtst-dev libx11-dev libevdev-dev && rm -rf /var/lib/apt/lists/*

# sccache (pre-installed so the action only needs to configure it)
RUN curl -fsSL https://github.com/mozilla/sccache/releases/download/v0.10.0/sccache-v0.10.0-x86_64-unknown-linux-musl.tar.gz \
    | tar xz -C /usr/local/bin --strip-components=1 sccache-v0.10.0-x86_64-unknown-linux-musl/sccache \
    && chmod +x /usr/local/bin/sccache

# tauri-driver (WebDriver server for Tauri E2E tests)
RUN cargo install tauri-driver --version 2.0.5

# Vendored CEF-aware tauri-cli. The upstream @tauri-apps/cli binary does not
# bundle CEF framework files into installers — only the fork at
# app/src-tauri/vendor/tauri-cef (a submodule) does. We compile it here so
# release/build workflows can invoke `cargo tauri build` directly without
# paying the ~5 min compile cost every run.
#
# Submodule must be checked out before `docker build` (see docker-ci-image.yml
# `submodules: recursive`). Source is discarded after install — only the
# `cargo-tauri` binary in $CARGO_HOME/bin is kept.
COPY app/src-tauri/vendor/tauri-cef /opt/tauri-cef
RUN cargo install --locked --path /opt/tauri-cef/crates/tauri-cli \
    && rm -rf /opt/tauri-cef /usr/local/cargo/registry/cache /usr/local/cargo/registry/src

# E2E entrypoint (starts Xvfb + dbus for headless webkit2gtk testing)
COPY e2e/docker-entrypoint.sh /docker-entrypoint.sh
RUN chmod +x /docker-entrypoint.sh

# Verify installs
RUN rustc --version && cargo --version && node --version && yarn --version && mold --version && sccache --version && which tauri-driver && cargo tauri --version
