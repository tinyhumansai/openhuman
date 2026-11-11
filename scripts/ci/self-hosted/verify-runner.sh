#!/usr/bin/env bash
# Fail closed on stale guests. Never install or silently select another compiler.
# `image` checks only the pinned digest against CI_IMAGE_BUILD_INFO (default
# /etc/ohci/build-info). With no argument, also check preinstalled tools and
# the pinned linker, then append the toolchain configuration to GITHUB_ENV.
set -euo pipefail
root="$(cd "$(dirname "$0")/../../.." && pwd)"
image="$(cat "$root/scripts/ci/self-hosted/guest-image.txt")"
metadata="${CI_IMAGE_BUILD_INFO:-/etc/ohci/build-info}"
if [[ ! "$image" =~ ^ghcr.io/tinyhumansai/rust_ci@sha256:[0-9a-f]{64}$ ]] ||
  [[ ! -r "$metadata" ]] || ! grep -Fxq "digest=${image##*@}" "$metadata"; then
  echo '::error::Private guest image metadata is missing or does not match guest-image.txt' >&2
  exit 1
fi
echo "[ci][guest] verified $image"
[[ "${1:-}" != image ]] || exit 0

for tool in rustup cargo rustc clang readelf python3 node pnpm jq git curl pkg-config pwsh diff-cover; do
  command -v "$tool" >/dev/null || { echo "::error::Missing preinstalled tool: $tool" >&2; exit 1; }
done
linker=/usr/local/bin/rust-mold-linker
[[ -x "$linker" && -x /opt/mold/bin/mold ]] || { echo '::error::Missing pinned mold linker' >&2; exit 1; }
channel="$(python3 - "$root/rust-toolchain.toml" <<'PYTHON'
import sys
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib
assert sys.version_info >= (3, 10)
print(tomllib.load(open(sys.argv[1], "rb"))["toolchain"]["channel"])
PYTHON
)"
# cargo-llvm-cov --version does not inspect LLVM tools. Check the pinned
# toolchain before publishing environment, and forbid its automatic installer.
verify_llvm_tools() {
  local toolchain="$1" target_libdir llvm_bin tool
  target_libdir="$(rustup run "$toolchain" rustc --print target-libdir)" || return $?
  llvm_bin="$(dirname "$target_libdir")/bin"
  for tool in llvm-cov llvm-profdata; do
    if [[ ! -x "$llvm_bin/$tool" ]]; then
      printf '::error::Missing preinstalled LLVM tool for %s: %s\n' "$toolchain" "$tool" >&2
      return 1
    fi
  done
  printf 'CARGO_LLVM_COV_SETUP=no\n' >> "$GITHUB_ENV"
}
verify_llvm_tools "$channel"
# rustup run --install is deliberately absent: an image without this toolchain fails.
rustup run "$channel" rustc --version
rustup run "$channel" cargo clippy --version
rustup run "$channel" cargo fmt --version
rustup run "$channel" cargo nextest --version
rustup run "$channel" cargo llvm-cov --version
/opt/mold/bin/mold --version
{
  echo "RUSTUP_TOOLCHAIN=$channel"
  echo "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=$linker"
  echo "OPENHUMAN_CI_PYTHON=$(command -v python3)"
} >> "$GITHUB_ENV"
