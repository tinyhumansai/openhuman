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
#
# Staging runs in a subshell, so sourcing leaves the caller's shell options
# alone; only the two exports come back. Any failure (an unresolvable module,
# a download, digest or extraction error) exports nothing and makes `source`
# return nonzero, with the reason on stderr. Each step checks its own result
# with `fail`: `set -e` is ignored inside the `if` that runs this function.

ts_e2e_stage() (
  set -uo pipefail
  fail() { echo "ts-e2e: $*" >&2; exit 1; }
  ids="${1:-tinysecurity,tinycomputer,tinyconnectors,tinybox,tinyflows}"
  cache="${OPENHUMAN_TS_E2E_CACHE:-$HOME/.cache/openhuman-ts-e2e}"
  bundled="$cache/bundled"
  mkdir -p "$cache" && chmod 700 "$cache" && rm -rf "$bundled" || fail "cannot prepare $cache"

  host_key="${OH_MODULE_HOST_KEY:-}"
  if [ -z "$host_key" ]; then
    case "$(uname -s)-$(uname -m)" in
      Linux-x86_64) host_key="ubuntu-24.04-x86_64" ;;
      Linux-aarch64 | Linux-arm64) host_key="ubuntu-24.04-arm64" ;;
      Darwin-arm64) host_key="macos-15-arm64" ;;
      Darwin-*) host_key="macos-15-x86_64" ;;
      *) fail "unsupported host $(uname -s) $(uname -m)" ;;
    esac
  fi

  assets="$(node scripts/ci/self-hosted/test-module-assets.mjs "$host_key" "$ids")" ||
    fail "cannot resolve the release assets of $ids for $host_key"
  [ -n "$assets" ] || fail "no release assets for $ids on $host_key"

  loadable=""
  while IFS=$'\t' read -r id url archive sha; do
    path="$cache/$archive"
    curl --fail --location --silent --show-error --retry 4 --retry-delay 2 "$url" --output "$path" ||
      fail "cannot download $archive"
    node - "$path" "$sha" <<'NODE' || fail "digest check failed for $archive; not extracting it"
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
    version="${version%%-"$host_key"*}"
    dir="$bundled/$id/$version/$host_key"
    mkdir -p "$dir" && cp "$path" "$dir/$archive" || fail "cannot stage $archive"
    tar --no-same-owner -xzf "$dir/$archive" -C "$dir" || fail "cannot extract $archive"
    if [[ "$(uname -s)" == Linux ]]; then
      if ! deps="$(find "$dir" -name '*.so' -exec ldd {} + 2>&1)"; then
        echo "ts-e2e: $id $version staged but not loadable here; ldd failed: $deps" >&2
        continue
      fi
      missing="$(printf '%s\n' "$deps" | awk '/not found/ {print $1}' | sort -u | tr '\n' ' ')"
      if [[ -n "$missing" ]]; then
        echo "ts-e2e: $id $version staged but not loadable here; runner lacks: $missing" >&2
        continue
      fi
    fi
    echo "ts-e2e: staged $id $version" >&2
    loadable="${loadable:+$loadable,}$id"
  done <<< "$assets"

  printf 'export OPENHUMAN_BUNDLED_MODULES=%q\nexport OPENHUMAN_TS_E2E_MODULES=%q\n' "$bundled" "$loadable"
)

if ts_e2e_exports="$(ts_e2e_stage "$@")"; then
  eval "$ts_e2e_exports"
  unset -f ts_e2e_stage
  unset ts_e2e_exports
  echo "ts-e2e: OPENHUMAN_BUNDLED_MODULES=$OPENHUMAN_BUNDLED_MODULES OPENHUMAN_TS_E2E_MODULES=$OPENHUMAN_TS_E2E_MODULES" >&2
else
  unset -f ts_e2e_stage
  unset ts_e2e_exports
  echo "ts-e2e: staging failed; nothing exported" >&2
  false
fi
