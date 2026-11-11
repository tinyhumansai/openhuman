#!/usr/bin/env bash
# Install a bubblewrap new enough for the TinyBox native jail into the CI
# container (Ubuntu 22.04), so sandbox and shell tests run against a real jail.
#
# TinySecurity plans every agent command onto a jail that enforces filesystem
# and network isolation, and TinyBox's probe needs `bwrap --disable-userns`
# (bubblewrap >= 0.8). Jammy ships 0.6.1 and has no newer package, so this
# builds the pinned upstream release, verified by checksum, and installs it as
# /usr/bin/bwrap. The job must also run the container with
# `--security-opt seccomp=unconfined --security-opt apparmor=unconfined
# --security-opt systempaths=unconfined` (see .github/workflows/ci-lanes.yml):
# Docker's defaults refuse the user namespace and mask /proc.
#
#   bash scripts/ci/install-bubblewrap.sh
set -euo pipefail

VERSION=0.11.0
SHA256=988fd6b232dafa04b8b8198723efeaccdb3c6aa9c1c7936219d5791a8b7a8646
# Ubuntu's archive carries the upstream release tarball byte-for-byte as the
# package's orig tarball, so the one pinned checksum verifies either source.
# It is tried first because GitHub's release CDN times out (504) under load;
# GitHub stays as the fallback.
URLS=(
  "http://archive.ubuntu.com/ubuntu/pool/main/b/bubblewrap/bubblewrap_${VERSION}.orig.tar.xz"
  "https://github.com/containers/bubblewrap/releases/download/v${VERSION}/bubblewrap-${VERSION}.tar.xz"
)

if command -v bwrap >/dev/null 2>&1 && bwrap --help 2>&1 | grep -q -- '--disable-userns'; then
  echo "[ci][bwrap] $(bwrap --version) already supports --disable-userns"
  exit 0
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "[ci][bwrap] installing build dependencies"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends \
  ca-certificates curl xz-utils gcc libc6-dev pkg-config libcap-dev meson ninja-build >/dev/null

echo "[ci][bwrap] fetching bubblewrap ${VERSION}"
fetched=""
for url in "${URLS[@]}"; do
  echo "[ci][bwrap] trying ${url}"
  if curl -fsSL --retry 3 --retry-all-errors --retry-delay 5 --connect-timeout 30 --max-time 300 \
    -o "$work/bubblewrap.tar.xz" "$url" &&
    echo "${SHA256}  $work/bubblewrap.tar.xz" | sha256sum -c - >/dev/null; then
    fetched="$url"
    break
  fi
  echo "[ci][bwrap] ${url} failed or did not match the pinned checksum" >&2
  rm -f "$work/bubblewrap.tar.xz"
done
if [ -z "$fetched" ]; then
  echo "[ci][bwrap] no source served bubblewrap ${VERSION} with the pinned checksum" >&2
  exit 1
fi
echo "[ci][bwrap] fetched and verified from ${fetched}"

tar -xJf "$work/bubblewrap.tar.xz" -C "$work"
src="$work/bubblewrap-${VERSION}"
meson setup "$work/build" "$src" --prefix=/usr --buildtype=release \
  -Dman=disabled -Dtests=false -Dselinux=disabled -Dbash_completion=disabled -Dzsh_completion=disabled >/dev/null
ninja -C "$work/build" >/dev/null
install -o root -g root -m 0755 "$work/build/bwrap" /usr/bin/bwrap

bwrap --version
if ! bwrap --help 2>&1 | grep -q -- '--disable-userns'; then
  echo "[ci][bwrap] installed bwrap lacks --disable-userns" >&2
  exit 1
fi
# Prove the container can actually create the namespaces the jail needs.
if ! bwrap --ro-bind / / --unshare-user --unshare-pid --unshare-net --disable-userns \
  --proc /proc --dev /dev --tmpfs /tmp true; then
  echo "[ci][bwrap] bwrap cannot create a jail here; check the container's security options" >&2
  exit 1
fi
echo "[ci][bwrap] jail probe ok"
