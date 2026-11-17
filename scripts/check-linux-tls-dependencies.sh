#!/usr/bin/env bash
set -euo pipefail

target="${1:-x86_64-unknown-linux-gnu}"
forbidden='^(aws-lc-sys|aws-lc-rs) v'

check_world() {
  local label="$1"
  local manifest="$2"
  local tree
  if ! tree="$(cargo tree --locked --manifest-path "$manifest" --target "$target" --prefix none)"; then
    printf 'error: could not check %s dependency tree for %s\n' "$label" "$target" >&2
    exit 2
  fi
  if matches="$(printf '%s\n' "$tree" | grep -E "$forbidden")"; then
    printf 'error: aws-lc dependencies found in %s for %s:\n%s\n' \
      "$label" "$target" "$matches" >&2
    exit 1
  fi

  # The core must use rustls only. Tauri has a deliberate native-TLS/OpenSSL
  # exception for its updater/dev proxy dependencies, checked by ownership
  # below so that only those known paths retain the exemption.
  if [[ "$label" == core ]] &&
    matches="$(printf '%s\n' "$tree" | grep -E '^(native-tls|openssl|openssl-sys) v')"; then
    printf 'error: native TLS/OpenSSL dependencies found in %s for %s:\n%s\n' \
      "$label" "$target" "$matches" >&2
    exit 1
  fi

  # Tauri legitimately owns reqwest 0.13 for its dev proxy/updater. Sentry
  # must never own that tree: OpenHuman supplies its reqwest 0.12 transport.
  #
  # A `while read` loop rather than `mapfile`, which needs bash 4: the stock
  # macOS /bin/bash is 3.2, where `mapfile` dies with exit 127 partway
  # through the run and reads like a policy failure. Guarding the first
  # element avoids expanding an empty array under bash 3.2's `set -u`.
  reqwest_013_versions=()
  while IFS= read -r version; do
    [[ -n "$version" ]] && reqwest_013_versions+=("$version")
  done < <(
    printf '%s\n' "$tree" |
      sed -nE 's/^reqwest v(0\.13\.[^ ]+).*/\1/p' |
      sort -u
  )
  if [[ ${reqwest_013_versions[0]+set} ]]; then
    for version in "${reqwest_013_versions[@]}"; do
      if ! owners="$(
        cargo tree --locked --manifest-path "$manifest" --target "$target" \
          --prefix none --invert "reqwest@$version"
      )"; then
        printf 'error: could not check reqwest %s owners in %s\n' "$version" "$label" >&2
        exit 2
      fi
      if grep -Eq '^sentry v' <<< "$owners"; then
        printf 'error: Sentry owns reqwest %s in %s\n%s\n' \
          "$version" "$label" "$owners" >&2
        exit 1
      fi
    done
  fi

  for package in native-tls openssl openssl-sys; do
    # Cargo returns an error for --invert when the package is absent.
    if ! grep -Eq "^${package} v" <<< "$tree"; then
      continue
    fi
    if ! owners="$(
      cargo tree --locked --manifest-path "$manifest" --target "$target" \
        --prefix none --invert "$package"
    )"; then
      printf 'error: could not check %s owners in %s\n' "$package" "$label" >&2
      exit 2
    fi
    if grep -Eq '^sentry v' <<< "$owners"; then
      printf 'error: Sentry owns %s in %s\n%s\n' \
        "$package" "$label" "$owners" >&2
      exit 1
    fi
  done
}

check_world core Cargo.toml
check_world tauri crates/openhuman-app/Cargo.toml

printf 'Linux TLS/Sentry dependency policy passed for both Cargo worlds (%s)\n' "$target"
