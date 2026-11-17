#!/usr/bin/env bash
# Check out the submodules a CI build needs, fetching in parallel.
#
# All top-level vendor/ submodules, plus only the nested submodules a Cargo
# build reads. The rest of the recursive tree (GitHub wikis and nested copies
# of tinybus that the root [patch] replaces with vendor/tinybus,
# and tinymemory's own tinycortex/tinyinference) is never compiled, so fetching
# it only costs checkout time. If a build starts to need another nested
# submodule, cargo fails with "failed to read .../Cargo.toml"; add it below.
#
#   bash scripts/ci/checkout-submodules.sh
set -euo pipefail
cd "$(dirname "$0")/../.."

# "<parent submodule> <nested path>" pairs a Cargo build reads.
NESTED=(
  "vendor/tinyagents vendor/tinytools"
  "vendor/tinyagents vendor/tinyinference"
  # tinyagents-live (the live voice agent runner) builds the vendored
  # tinyliveagents library by path.
  "vendor/tinyagents vendor/tinyliveagents"
  # The native TinyConnectors test module is built from its own manifest, so
  # the root workspace's tinybus patch does not apply to this dependency.
  "vendor/tinyconnectors vendor/tinybus"
  # openhuman-modules-linked names every module crate as an optional
  # dependency, so cargo resolves their manifests in every build. TinyComputer's
  # browser and desktop crates reach these by path.
  "vendor/tinycomputer vendor/agent-browser"
  "vendor/tinycomputer vendor/agent-desktop"
  "vendor/tinycomputer vendor/tinyinference"
)
JOBS="${SUBMODULE_JOBS:-8}"

git -c protocol.version=2 submodule update --init --jobs "${JOBS}"
for pair in "${NESTED[@]}"; do
  read -r parent nested <<<"${pair}"
  git -C "${parent}" -c protocol.version=2 submodule update --init --jobs "${JOBS}" -- "${nested}"
done
echo "[ci][submodules] checked out $(git submodule status | wc -l) top-level and ${#NESTED[@]} nested submodules"
