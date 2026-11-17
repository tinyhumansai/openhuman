#!/usr/bin/env bash
# Make sure the toolchain pinned in rust-toolchain.toml is installed and that
# its `cargo` and `rustc` actually run, retrying a broken download.
#
# The container image is rebuilt only when ghcr accepts the push, so it can lag
# the pin (it shipped 1.96.1 while the pin said 1.99.1). Every job then lets
# rustup auto-install the pin on the first cargo call, and a flaky download
# leaves a half-installed toolchain: "the 'cargo' binary ... is not applicable
# to the '1.99.1-...' toolchain". Doing the install here, once, with a retry and
# a clean uninstall between attempts, turns that flake into a retry.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
channel="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$root/rust-toolchain.toml" | head -n1)"
if [ -z "$channel" ]; then
  echo "[install-pinned-toolchain] no channel in rust-toolchain.toml" >&2
  exit 1
fi

# Install with the rustup component names (`-c`); `component list` prints the
# target-suffixed names (llvm-tools-<target>), so the health check matches prefixes.
components="rustfmt clippy llvm-tools-preview"
installed_prefixes="rustfmt clippy llvm-tools"

# A toolchain the image already ships is reused as-is: a failed download must
# not uninstall a working pin. Healthy means cargo and rustc run from the repo
# and every required component is installed.
toolchain_usable() {
  local installed p
  installed="$(cd "$root" && rustup component list --toolchain "$channel" --installed 2>/dev/null)" || return 1
  for p in $installed_prefixes; do
    grep -q "^$p" <<<"$installed" || return 1
  done
  (cd "$root" && cargo --version && rustc --version) >/dev/null 2>&1
}

if toolchain_usable; then
  echo "[install-pinned-toolchain] $channel already usable; reusing it"
  exit 0
fi

component_flags=()
for c in $components; do component_flags+=(-c "$c"); done

attempts="${INSTALL_TOOLCHAIN_ATTEMPTS:-3}"
for attempt in $(seq 1 "$attempts"); do
  echo "[install-pinned-toolchain] $channel attempt $attempt/$attempts"
  if (cd "$root" && rustup toolchain install "$channel" --profile minimal "${component_flags[@]}" --no-self-update \
      && toolchain_usable); then
    exit 0
  fi
  rustup toolchain uninstall "$channel" || true
  sleep $((attempt * 5))
done
echo "[install-pinned-toolchain] $channel could not be installed after $attempts attempts" >&2
exit 1
