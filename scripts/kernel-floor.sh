#!/usr/bin/env bash
# Measure the dependency floor of a build profile.
#
# The kernel-profile invariant: `--no-default-features --features flows` is the
# surface a second host would embed, and it must not grow. This script is the
# metric behind that ratchet (see `scripts/check-kernel-floor.sh` and the CI job).
#
# Three numbers, because they answer different questions and are easy to confuse:
#
#   packages  name+version lines. Two major versions of one crate count twice.
#             This is compile units — what you pay in build time.
#   names     unique crate names. Duplicated majors collapse. This is supply
#             chain surface — how many distinct projects you trust.
#   native    crates with a build script that shells out to a C/C++/asm
#             toolchain. This is what makes a build need cc/cmake/pkg-config.
#
# `cargo tree` prints ` (*)` on a subtree it already expanded. Those lines are
# back-references, NOT additional dependencies — counting them inflates the
# total (the 456-vs-454 discrepancy in the original audit was exactly this).
# We strip the marker and deduplicate.
#
# Usage:
#   scripts/kernel-floor.sh                    # default features
#   scripts/kernel-floor.sh flows              # --no-default-features --features flows
#   scripts/kernel-floor.sh flows --json
#   scripts/kernel-floor.sh --all-features
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

JSON=0
FEATURES=""
for arg in "$@"; do
  case "$arg" in
    --json) JSON=1 ;;
    --*)    FEATURES="${FEATURES:+$FEATURES }$arg" ;;
    *)      FEATURES="${FEATURES:+$FEATURES }$arg" ;;
  esac
done

# Build the cargo argument list. A bare feature list implies
# --no-default-features, because measuring "default + flows" is never what a
# floor question is asking.
declare -a CARGO_ARGS=(tree -e normal --prefix none)
PROFILE_LABEL="default"
if [[ -n "$FEATURES" ]]; then
  if [[ "$FEATURES" == --* ]]; then
    # shellcheck disable=SC2206
    CARGO_ARGS+=($FEATURES)
    PROFILE_LABEL="$FEATURES"
  else
    CARGO_ARGS+=(--no-default-features --features "$FEATURES")
    PROFILE_LABEL="no-default + $FEATURES"
  fi
fi

# GGML_NATIVE=OFF matches the documented macOS-Apple-Silicon workaround and is
# harmless elsewhere; without it this can fail on some hosts before it ever
# resolves the graph.
TREE="$(GGML_NATIVE=OFF cargo "${CARGO_ARGS[@]}" 2>/dev/null | sed 's/ (\*)$//' | grep -vE '^[[:space:]]*$' || true)"

if [[ -z "$TREE" ]]; then
  echo "kernel-floor: cargo tree produced no output for: ${CARGO_ARGS[*]}" >&2
  exit 1
fi

PACKAGES="$(printf '%s\n' "$TREE" | sort -u | wc -l | tr -d ' ')"
NAMES="$(printf '%s\n' "$TREE" | awk '{print $1}' | sort -u | wc -l | tr -d ' ')"

# Native builds: crates known to invoke a C/C++/asm toolchain from build.rs.
# Matched by name against the resolved graph rather than by parsing build
# scripts, because the set is small, stable, and the parse is not worth it.
NATIVE_CANDIDATES='^(libsqlite3-sys|libgit2-sys|libz-sys|lzma-sys|aws-lc-sys|ring|whisper-rs-sys|openssl-sys|zstd-sys|bzip2-sys|curl-sys|onig_sys|tree-sitter)$'
NATIVE_LIST="$(printf '%s\n' "$TREE" | awk '{print $1}' | sort -u | grep -E "$NATIVE_CANDIDATES" || true)"
NATIVE="$(printf '%s' "$NATIVE_LIST" | grep -c . || true)"

if [[ "$JSON" == "1" ]]; then
  printf '{"profile":"%s","packages":%s,"names":%s,"native":%s,"native_crates":[' \
    "$PROFILE_LABEL" "$PACKAGES" "$NAMES" "$NATIVE"
  first=1
  while IFS= read -r c; do
    [[ -z "$c" ]] && continue
    [[ $first == 1 ]] || printf ','
    printf '"%s"' "$c"
    first=0
  done <<< "$NATIVE_LIST"
  printf ']}\n'
else
  echo "profile:  $PROFILE_LABEL"
  echo "packages: $PACKAGES   (name+version, '(*)' back-refs stripped)"
  echo "names:    $NAMES   (unique crate names)"
  echo "native:   $NATIVE   (crates with a C/C++/asm build)"
  if [[ -n "$NATIVE_LIST" ]]; then
    while IFS= read -r c; do
      [[ -z "$c" ]] && continue
      printf '          %s\n' "$c"
    done <<< "$NATIVE_LIST"
  fi
fi
