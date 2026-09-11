#!/usr/bin/env bash
# Shared Tauri updater-signing helper for the release scripts.
#
# Sourced, not executed — it only defines `tauri_signer_sign` and deliberately
# sets no shell options, so it inherits the caller's `set -euo pipefail`.
#
# Why this exists (#5658): the release lane builds the desktop app with the npm
# `@tauri-apps/cli` — `pnpm tauri build` in .github/workflows/build-desktop.yml
# — and never installs the `cargo-tauri` crate. Both release scripts none the
# less reached for `cargo tauri signer sign`, the one form that is not present
# on the runner:
#
#   * upload-macos-artifacts.sh called it unguarded, so every Release Production
#     run died with `error: no such command: tauri` (exit 101) at the final
#     upload step of a ~50-minute job, after the DMG had already been replaced.
#   * strip-appimage-graphics-libs.sh guarded it with `command -v cargo-tauri`
#     and returned on failure. That guard has always been taken, so the Linux
#     leg silently published an updater tarball whose .sig still covered the
#     pre-strip bytes — a signature that cannot verify on an installed client.
#
# Resolution order below prefers the CLI the lane actually installs, and the
# helper FAILS instead of skipping when no signer is available. An updater
# artifact published without a valid signature is worse than a failed job:
# clients verify the signature, so a bad .sig breaks updates for everyone who
# already has the app, and it does so silently at release time.

# Sign "$1" with the Tauri updater key, writing "$1.sig" alongside it.
#
# Requires TAURI_SIGNING_PRIVATE_KEY to be set; TAURI_SIGNING_PRIVATE_KEY_PASSWORD
# is optional and may be empty for an unencrypted key. The password is always
# passed explicitly so an encrypted key cannot stall a non-interactive runner on
# a passphrase prompt.
#
# Returns 0 only when a non-empty .sig was produced; non-zero otherwise. Callers
# that legitimately have nothing to sign (an unsigned PR build with no key
# configured) must decide that *before* calling this.
tauri_signer_sign() {
  local file="$1"
  local key="${TAURI_SIGNING_PRIVATE_KEY:-}"
  local password="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"

  if [ -z "$key" ]; then
    echo "[tauri-signer] ERROR: TAURI_SIGNING_PRIVATE_KEY is not set; refusing to publish an unsigned updater artifact: $file" >&2
    return 1
  fi
  if [ ! -f "$file" ]; then
    echo "[tauri-signer] ERROR: nothing to sign, no such file: $file" >&2
    return 1
  fi

  local script_dir repo_root app_dir app_bin
  script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  repo_root="$(cd "$script_dir/../.." && pwd)"
  app_dir="$repo_root/app"
  app_bin="$app_dir/node_modules/.bin/tauri"

  # Drop any stale signature first so the check below cannot pass on a leftover
  # .sig from an earlier bundle. This is the Linux failure mode exactly.
  rm -f "$file.sig"

  local sign_status=0
  if [ -x "$app_bin" ]; then
    # The binary `pnpm install` puts in the workspace — what `pnpm tauri build`
    # resolves to in build-desktop.yml. Invoked directly so neither pnpm nor a
    # package.json script indirection can swallow `--private-key`.
    "$app_bin" signer sign --private-key "$key" --password "$password" "$file" || sign_status=$?
  elif command -v pnpm >/dev/null 2>&1 && [ -d "$app_dir" ]; then
    # Same CLI, reached through the workspace when the .bin shim is not where we
    # expect it (a different hoisting layout, for instance). Must run *from*
    # app/: `pnpm --dir <path> exec` takes pnpm's recursive path and fails with
    # ERR_PNPM_RECURSIVE_EXEC_FIRST_FAIL instead.
    ( cd "$app_dir" && pnpm exec tauri signer sign --private-key "$key" --password "$password" "$file" ) || sign_status=$?
  elif command -v cargo-tauri >/dev/null 2>&1; then
    # Only for a developer machine that happens to have the cargo subcommand
    # installed. CI never has it — that is the bug this file exists to fix.
    cargo-tauri signer sign --private-key "$key" --password "$password" "$file" || sign_status=$?
  else
    echo "[tauri-signer] ERROR: no Tauri CLI available to sign $file" >&2
    echo "[tauri-signer]   looked for: $app_bin" >&2
    echo "[tauri-signer]               (cd $app_dir && pnpm exec tauri)" >&2
    echo "[tauri-signer]               cargo-tauri on PATH" >&2
    echo "[tauri-signer]   The release lane installs the npm CLI with 'pnpm install'; see .github/workflows/build-desktop.yml." >&2
    return 1
  fi

  if [ "$sign_status" -ne 0 ]; then
    echo "[tauri-signer] ERROR: Tauri signer exited with status $sign_status for $file" >&2
    return 1
  fi
  if [ ! -s "$file.sig" ]; then
    echo "[tauri-signer] ERROR: signer reported success but $file.sig is missing or empty" >&2
    return 1
  fi

  echo "[tauri-signer] Signed $(basename "$file")"
}
