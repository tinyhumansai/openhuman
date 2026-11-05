#!/usr/bin/env bash
# Stage registry-pinned module releases for the TinySecurity E2E fixtures (TS-E2E X6).
#
# Usage: source scripts/ci/ts-e2e-stage-modules.sh [id,...]
#   default ids: tinysecurity,tinycomputer,tinyconnectors,tinybox,tinyflows
#
# Each archive is downloaded from the URL the compiled registry pins, checked
# against its sha256, and laid out the way an installer bundle is
# (`<dir>/<id>/<version>/<host_key>/`, as security-native-fixture.sh does), so
# callback-bound modules load through the shipped release path.
#
# Exports:
#   OPENHUMAN_BUNDLED_MODULES   the bundle directory
#   OPENHUMAN_TS_E2E_MODULES    comma list of the ids that can load on this runner
# A Linux module linking a system library the runner lacks (TinyComputer needs
# libxdo and libasound) is left out of OPENHUMAN_TS_E2E_MODULES with a message;
# tests that need it fail with that reason instead of a dlopen error.
set -euo pipefail

ts_e2e_ids="${1:-tinysecurity,tinycomputer,tinyconnectors,tinybox,tinyflows}"
ts_e2e_cache="${OPENHUMAN_TS_E2E_CACHE:-$HOME/.cache/openhuman-ts-e2e}"
ts_e2e_bundled="$ts_e2e_cache/bundled"
mkdir -p "$ts_e2e_cache"
chmod 700 "$ts_e2e_cache"
rm -rf "$ts_e2e_bundled"

ts_e2e_host_key="${OH_MODULE_HOST_KEY:-}"
if [ -z "$ts_e2e_host_key" ]; then
  case "$(uname -s)" in
    Linux) ts_e2e_host_key="ubuntu-24.04-x86_64" ;;
    Darwin)
      case "$(uname -m)" in
        arm64) ts_e2e_host_key="macos-15-arm64" ;;
        *) ts_e2e_host_key="macos-15-x86_64" ;;
      esac
      ;;
    *) echo "ts-e2e: unsupported host $(uname -s)" >&2; return 1 2>/dev/null || exit 1 ;;
  esac
fi

ts_e2e_loadable=""
while IFS=$'\t' read -r id url archive sha; do
  path="$ts_e2e_cache/$archive"
  curl --fail --location --silent --show-error --retry 4 --retry-delay 2 "$url" --output "$path"
  node - "$path" "$sha" <<'NODE'
const { createHash } = require("node:crypto");
const { readFileSync } = require("node:fs");
const [file, expected] = process.argv.slice(2);
const actual = createHash("sha256").update(readFileSync(file)).digest("hex");
if (actual !== expected) {
  throw new Error(`ts-e2e module digest mismatch for ${file}: expected ${expected}, got ${actual}`);
}
NODE
  version="${archive#"$id"-module-}"
  version="${version#"$id"-}"
  version="${version%%-"$ts_e2e_host_key"*}"
  dir="$ts_e2e_bundled/$id/$version/$ts_e2e_host_key"
  mkdir -p "$dir"
  cp "$path" "$dir/$archive"
  tar --no-same-owner -xzf "$dir/$archive" -C "$dir"
  if [[ "$(uname -s)" == Linux ]]; then
    missing="$(find "$dir" -name '*.so' -exec ldd {} + | awk '/not found/ {print $1}' | sort -u | tr '\n' ' ')"
    if [[ -n "$missing" ]]; then
      echo "ts-e2e: $id $version staged but not loadable here; runner lacks: $missing" >&2
      continue
    fi
  fi
  echo "ts-e2e: staged $id $version" >&2
  ts_e2e_loadable="${ts_e2e_loadable:+$ts_e2e_loadable,}$id"
done < <(node scripts/ci/self-hosted/test-module-assets.mjs "$ts_e2e_host_key" "$ts_e2e_ids")

export OPENHUMAN_BUNDLED_MODULES="$ts_e2e_bundled"
export OPENHUMAN_TS_E2E_MODULES="$ts_e2e_loadable"
echo "ts-e2e: OPENHUMAN_BUNDLED_MODULES=$OPENHUMAN_BUNDLED_MODULES OPENHUMAN_TS_E2E_MODULES=$OPENHUMAN_TS_E2E_MODULES" >&2
