#!/usr/bin/env bash
# Resolve the native test-module archive from rustc's compilation target.
# `uname -m` reports x86_64 when an ARM Mac launches a translated shell, while
# the Rust toolchain may still build an aarch64 test binary.
set -euo pipefail

host_triple="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
macos_major="${2:-}"

case "$host_triple" in
  aarch64-apple-darwin|x86_64-apple-darwin)
    [ -n "$macos_major" ] || macos_major="$(sw_vers -productVersion | cut -d. -f1)"
    if [ "$macos_major" -ge 26 ]; then
      module_version=macos-26
    else
      module_version=macos-15
    fi
    case "$host_triple" in
      aarch64-apple-darwin) module_arch=arm64 ;;
      x86_64-apple-darwin) module_arch=x86_64 ;;
    esac
    printf 'dylib\t%s-%s\n' "$module_version" "$module_arch"
    ;;
  x86_64-unknown-linux-gnu)
    printf 'so\tubuntu-22.04-x86_64\n'
    ;;
  aarch64-unknown-linux-gnu)
    printf 'so\tubuntu-22.04-arm64\n'
    ;;
  *)
    echo "Unsupported native-module test target: $host_triple" >&2
    exit 1
    ;;
esac
