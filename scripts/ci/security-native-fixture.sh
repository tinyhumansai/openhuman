#!/usr/bin/env bash
# Exercise the released module through the compiled registry and archive digest.
# For explicit local module development, set OPENHUMAN_TEST_SECURITY_MODULE.
set -euo pipefail

if [[ -z "${OPENHUMAN_TEST_SECURITY_MODULE:-}" ]]; then
  case "$(uname -s)" in
    Darwin)
      case "$(uname -m)" in
        arm64) security_host="macos-15-arm64" ;;
        x86_64) security_host="macos-15-x86_64" ;;
        *) echo "Unsupported macOS architecture" >&2; exit 1 ;;
      esac ;;
    Linux)
      case "$(uname -m)" in
        aarch64) security_host="ubuntu-22.04-arm64" ;;
        x86_64) security_host="ubuntu-22.04-x86_64" ;;
        *) echo "Unsupported Linux architecture" >&2; exit 1 ;;
      esac ;;
    MINGW* | MSYS* | CYGWIN*) security_host="windows-2025-x86_64" ;;
    *) echo "Unsupported native security fixture host" >&2; exit 1 ;;
  esac
  export OPENHUMAN_TEST_SECURITY_RELEASE_HOST="${OPENHUMAN_TEST_SECURITY_RELEASE_HOST:-$security_host}"
fi

export OPENHUMAN_TEST_SECURITY_FIXTURE_DIR="$HOME/.cache/openhuman-security-fixtures"
mkdir -p "$OPENHUMAN_TEST_SECURITY_FIXTURE_DIR"

bash scripts/ci-cancel-aware.sh cargo test -p openhuman --lib \
  --no-default-features --features inference,web3,modules,security-module \
  modules::security -- --include-ignored --nocapture
