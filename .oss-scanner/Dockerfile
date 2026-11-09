# Keep source, compilers and dependency caches for the scanner's offline audit.
FROM node:24-bookworm AS node
FROM rust:1.99.1-bookworm

ENV DEBIAN_FRONTEND=noninteractive \
    CARGO_BUILD_JOBS=2 \
    CARGO_INCREMENTAL=0 \
    CARGO_PROFILE_DEV_DEBUG=line-tables-only \
    RUSTFLAGS="-C force-frame-pointers=yes"

RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential ca-certificates clang cmake curl git libasound2-dev \
      libclang-dev libevdev-dev libssl-dev libx11-dev libxdo-dev libxtst-dev \
      pkg-config python3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && npm install --global pnpm@10.10.0

COPY . /src
WORKDIR /src

# The scanner clones the top-level repository without recursive submodules.
RUN git submodule update --init --recursive --depth 1 --jobs 4
RUN pnpm install --frozen-lockfile
RUN cargo fetch --locked
RUN bash scripts/ci-cancel-aware.sh cargo build --locked -p openhuman-cli --bin openhuman-core
RUN pnpm build

# Fetch and verify native modules now; audit-time networking is disabled.
ENV OPENHUMAN_BUNDLED_MODULES=/src/target/oss-scanner-modules
RUN node scripts/release/stage-modules.mjs --output /src/target/oss-scanner-modules

# Leave unit-test tooling and caches in the image. No real service credentials
# are needed: service-dependent scenarios should use the repository's mocks.
# The default one-worker suite takes 27 minutes, exhausting tools/check's
# 45-minute clone/build/export budget. Use the project-supported CI settings
# and run the complete suite; leave failures visible.
RUN VITEST_MAX_WORKERS=2 VITEST_POOL=threads pnpm test
