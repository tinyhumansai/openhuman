##
# Linux E2E environment for OpenHuman desktop app.
#
# Mirrors the ubuntu-22.04 CI runner so macOS developers can run the
# same Linux-based E2E stack locally via Docker.
#
# Usage:
#   docker compose -f e2e/docker-compose.yml run --rm e2e
#
# Or manually:
#   docker build -t openhuman-e2e -f e2e/Dockerfile .
#   docker run --rm -v "$(pwd):/app" openhuman-e2e yarn workspace openhuman-app test:e2e:all
#
FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive

# System dependencies for Tauri + webkit2gtk
RUN apt-get update && apt-get install -y \
  curl build-essential pkg-config \
  libgtk-3-dev libwebkit2gtk-4.1-dev libappindicator3-dev \
  librsvg2-dev patchelf libssl-dev \
  xvfb at-spi2-core dbus-x11 webkit2gtk-driver \
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
RUN cargo install tauri-driver

WORKDIR /app

COPY e2e/docker-entrypoint.sh /docker-entrypoint.sh
RUN chmod +x /docker-entrypoint.sh

ENTRYPOINT ["/docker-entrypoint.sh"]
CMD ["yarn", "workspace", "openhuman-app", "test:e2e:all"]
