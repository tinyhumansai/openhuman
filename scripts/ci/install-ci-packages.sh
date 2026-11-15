#!/usr/bin/env bash
# Install the tools the container lanes need but the CI image does not carry:
# jq and tomli for the release-script tests, and unzip, which stage-modules.mjs
# unpacks module archives with.
#
# The image's apt sources point at archive.ubuntu.com, and the hosted runners
# sometimes cannot resolve it: `apt-get update` then "succeeds" with no index
# and the install fails with "Unable to locate package". So each install is
# retried, and after a failed attempt the sources move to the next mirror. A
# host that already has every tool makes no apt call at all.
#
#   bash scripts/ci/install-ci-packages.sh
#
# CI_APT_SOURCES and CI_APT_RETRY_DELAY exist for the script's own test.
set -euo pipefail

PACKAGES=(jq python3-tomli unzip)
SOURCES="${CI_APT_SOURCES:-/etc/apt/sources.list}"
DELAY="${CI_APT_RETRY_DELAY:-5}"
# archive.ubuntu.com as shipped, then mirrors carrying the same suites
# (including -security).
MIRRORS=(azure.archive.ubuntu.com mirrors.edge.kernel.org)

have_tools() {
  command -v jq >/dev/null 2>&1 &&
    command -v unzip >/dev/null 2>&1 &&
    python3 -c 'import tomli' >/dev/null 2>&1
}

attempt() {
  apt-get update || true
  apt-get install -y --no-install-recommends "${PACKAGES[@]}"
}

use_mirror() {
  local mirror=$1
  echo "[ci][apt] switching apt sources to ${mirror}"
  sed -i -E "s#https?://[a-z.]*(archive|security)\.ubuntu\.com/ubuntu#http://${mirror}/ubuntu#g; s#https?://mirrors\.edge\.kernel\.org/ubuntu#http://${mirror}/ubuntu#g" "$SOURCES"
}

if have_tools; then
  echo "[ci][apt] jq, tomli and unzip are already installed"
  exit 0
fi

if attempt; then
  exit 0
fi
for mirror in "${MIRRORS[@]}"; do
  echo "[ci][apt] install failed; retrying in ${DELAY}s via ${mirror}" >&2
  sleep "$DELAY"
  use_mirror "$mirror"
  if attempt; then
    exit 0
  fi
done
echo "[ci][apt] could not install ${PACKAGES[*]} from any mirror" >&2
exit 1
