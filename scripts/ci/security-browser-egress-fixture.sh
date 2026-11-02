#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
case "$(uname -s)" in
  Linux) ;;
  *) echo "The browser egress fixture runs on Linux only" >&2; exit 1 ;;
esac

cd -- "$repo_root"
export OPENHUMAN_TEST_SECURITY_FIXTURE_DIR="${OPENHUMAN_TEST_SECURITY_FIXTURE_DIR:-$HOME/.cache/openhuman-security-fixtures}"
features="$(scripts/ci/product-features.sh | sed 's/,os-keyring//')"
bash scripts/ci-cancel-aware.sh cargo test --manifest-path Cargo.toml -p openhuman --lib \
  --no-default-features --features "$features" \
  modules::security::tests::native_fixture_loads_the_pinned_release_and_reads_policy_info \
  -- --ignored --exact
bash scripts/ci-cancel-aware.sh cargo test --manifest-path Cargo.toml -p openhuman --lib \
  --no-default-features --features "$features" \
  modules::browser_egress_host::tests::browser_egress_callback

computer_dir="$repo_root/vendor/tinycomputer"
(
  cd -- "$computer_dir"
  bash "$repo_root/scripts/ci-cancel-aware.sh" cargo build -p tinycomputer-browser --features test-connector
)
