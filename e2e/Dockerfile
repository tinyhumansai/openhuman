##
# DEPRECATED: E2E tests now use the shared CI image from GHCR.
#
# The CI image (.github/Dockerfile) includes all E2E dependencies
# (xvfb, dbus, tauri-driver, webkit2gtk-driver).
#
# Usage:
#   docker compose -f e2e/docker-compose.yml run --rm e2e
#
# To build the CI image locally (if you can't pull from GHCR):
#   docker build -t openhuman-ci -f .github/Dockerfile .
#   IMAGE=openhuman-ci docker compose -f e2e/docker-compose.yml run --rm e2e
#
FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive

# System dependencies for Tauri + webkit2gtk
RUN apt-get update && apt-get install -y \
  bash curl build-essential pkg-config \
  libgtk-3-dev libwebkit2gtk-4.1-dev libappindicator3-dev \
  librsvg2-dev patchelf libssl-dev \
  libasound2-dev libxdo-dev libxtst-dev libx11-dev libevdev-dev \
  xvfb at-spi2-core dbus-x11 webkit2gtk-driver \
  clang libclang-dev cmake \
  git ca-certificates \
  && rm -rf /var/lib/apt/lists/*

# Rust 1.93.0
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
  sh -s -- -y --default-toolchain 1.93.0
ENV PATH="/root/.cargo/bin:${PATH}"

# Node.js 24.x + Yarn
RUN curl -fsSL https://deb.nodesource.com/setup_24.x | bash - && \
  apt-get install -y nodejs && \
  npm install -g yarn && \
  rm -rf /var/lib/apt/lists/*

# tauri-driver (WebDriver server for Tauri apps)
RUN cargo install tauri-driver --version 2.0.5

WORKDIR /app

ENTRYPOINT ["/docker-entrypoint.sh"]
CMD ["yarn", "workspace", "openhuman-app", "test:e2e:all"]
