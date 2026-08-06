#!/usr/bin/env bash
set -euo pipefail

target="${1:-x86_64-unknown-linux-gnu}"
forbidden='^(native-tls|openssl|openssl-sys) v'

check_world() {
  local label="$1"
  local manifest="$2"
  local tree
  tree="$(cargo tree --locked --manifest-path "$manifest" --target "$target" --prefix none)"
  if [[ "$label" == core ]] &&
    matches="$(printf '%s\n' "$tree" | grep -E "$forbidden")"; then
    printf 'error: native TLS/OpenSSL dependencies found in %s for %s:\n%s\n' \
      "$label" "$target" "$matches" >&2
    exit 1
  fi

  # Tauri legitimately owns reqwest 0.13 for its dev proxy/updater. Sentry
  # must never own that tree: OpenHuman supplies its reqwest 0.12 transport.
  mapfile -t reqwest_013_versions < <(
    printf '%s\n' "$tree" |
      sed -nE 's/^reqwest v(0\.13\.[^ ]+).*/\1/p' |
      sort -u
  )
  for version in "${reqwest_013_versions[@]}"; do
    owners="$(
      cargo tree --locked --manifest-path "$manifest" --target "$target" \
        --invert "reqwest@$version" 2>/dev/null || true
    )"
    if printf '%s\n' "$owners" | grep -Eq '^sentry v'; then
      printf 'error: Sentry owns reqwest %s in %s\n%s\n' \
        "$version" "$label" "$owners" >&2
      exit 1
    fi
  done

  for package in native-tls openssl openssl-sys; do
    owners="$(
      cargo tree --locked --manifest-path "$manifest" --target "$target" \
        --invert "$package" 2>/dev/null || true
    )"
    if printf '%s\n' "$owners" | grep -Eq '^sentry v'; then
      printf 'error: Sentry owns %s in %s\n%s\n' \
        "$package" "$label" "$owners" >&2
      exit 1
    fi
    if [[ "$label" == tauri ]] &&
      printf '%s\n' "$owners" | grep -Eq '^motosan-ai-oauth v'; then
      printf 'error: motosan-ai-oauth owns %s in %s\n%s\n' \
        "$package" "$label" "$owners" >&2
      exit 1
    fi
  done
}

check_world core Cargo.toml
check_world tauri app/src-tauri/Cargo.toml

printf 'Linux TLS/Sentry dependency policy passed for both Cargo worlds (%s)\n' "$target"
