#!/usr/bin/env bash
# Generate and upload latest.json for the Tauri auto-updater.
#
# Tauri's updater fetches a JSON manifest at a fixed endpoint (configured in
# app/src-tauri/tauri.conf.json via `plugins.updater.endpoints`), reads the
# `version` field, compares to the running app, and — if newer — downloads the
# platform-specific `url` and verifies it against `signature`.
#
# We host the manifest on the GitHub release itself. The endpoint in
# `prepareTauriConfig.js` resolves to
# `https://github.com/<repo>/releases/latest/download/latest.json`, which
# github permanently redirects to the asset on the newest non-draft release.
#
# Required env:
#   TAG          — the release tag (e.g. `v0.52.21`)
#   VERSION      — bare version (e.g. `0.52.21`)
#   REPO         — `owner/name` on github
#   GITHUB_TOKEN — with release write scope (for `gh release`)
#
# Signature files (`.sig` — base64 minisign detached signatures produced by
# the Tauri bundler when `createUpdaterArtifacts = true`) are downloaded from
# the release; the matching bundle URLs use the stable
# `/releases/download/<tag>/<file>` form so the manifest is self-describing.
set -euo pipefail

: "${TAG:?TAG required (e.g. v0.52.21)}"
: "${VERSION:?VERSION required (e.g. 0.52.21)}"
: "${REPO:?REPO required (e.g. tinyhumansai/openhuman)}"
: "${GITHUB_TOKEN:?GITHUB_TOKEN required}"

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

echo "[updater] Fetching asset list for $REPO $TAG"
gh release view "$TAG" --repo "$REPO" --json assets \
  --jq '.assets[].name' > "$WORKDIR/asset-names.txt"

asset_url() {
  printf 'https://github.com/%s/releases/download/%s/%s\n' "$REPO" "$TAG" "$1"
}

# Find a single asset matching the given extended regex on the release; echo
# its name on stdout, or empty if none / multiple. We prefer unambiguous
# matches and surface the asset list on failure.
find_asset() {
  local pattern="$1"
  local matches
  matches=$(grep -E "$pattern" "$WORKDIR/asset-names.txt" || true)
  local count
  count=$(printf '%s\n' "$matches" | grep -c . || true)
  if [ "$count" = "0" ]; then
    return 0
  fi
  if [ "$count" -gt "1" ]; then
    echo "[updater] WARN: pattern '$pattern' matched $count assets:" >&2
    printf '  %s\n' "$matches" >&2
    echo "[updater] WARN: using the first match" >&2
  fi
  printf '%s\n' "$matches" | head -1
}

# Download a .sig for an asset and echo the signature payload. The .sig file
# is a single base64-encoded minisign signature — no trimming needed beyond
# the trailing newline.
read_sig() {
  local name="$1"
  local sig_name="${name}.sig"
  if ! grep -Fxq "$sig_name" "$WORKDIR/asset-names.txt"; then
    echo "[updater] ERROR: signature asset '$sig_name' not on release — did createUpdaterArtifacts produce it?" >&2
    return 1
  fi
  gh release download "$TAG" --repo "$REPO" --pattern "$sig_name" \
    --dir "$WORKDIR" --clobber >&2
  # minisign sig format is two lines: an untrusted comment then the base64
  # payload. Tauri expects the whole file verbatim.
  local path="$WORKDIR/$sig_name"
  if [ ! -s "$path" ]; then
    echo "[updater] ERROR: downloaded sig is empty: $path" >&2
    return 1
  fi
  cat "$path"
}

# Platform mapping. Tauri's updater consults these exact keys; see
# https://v2.tauri.app/plugin/updater/#static-json-file
#
#   darwin-aarch64   — macOS Apple Silicon
#   darwin-x86_64    — macOS Intel
#   linux-x86_64     — Linux glibc x64 (AppImage)
#   windows-x86_64   — Windows x64 (NSIS setup)
#
# Naming conventions emitted by tauri-bundler with createUpdaterArtifacts:
#   darwin  : <AppName>_<version>_<arch>.app.tar.gz
#   linux   : <AppName>_<version>_amd64.AppImage
#   windows : <AppName>_<version>_x64-setup.exe
MAC_AARCH64=$(find_asset "^OpenHuman(_| ).*aarch64(-apple-darwin)?\.app\.tar\.gz$")
MAC_X86_64=$(find_asset  "^OpenHuman(_| ).*(x64|x86_64)(-apple-darwin)?\.app\.tar\.gz$")
LIN_X86_64=$(find_asset  "^OpenHuman(_| ).*amd64\.AppImage$")
WIN_X86_64=$(find_asset "^OpenHuman(_| ).*x64-setup\.exe$")

echo "[updater] Resolved updater bundles:"
echo "  darwin-aarch64  = ${MAC_AARCH64:-<missing>}"
echo "  darwin-x86_64   = ${MAC_X86_64:-<missing>}"
echo "  linux-x86_64    = ${LIN_X86_64:-<missing>}"
echo "  windows-x86_64  = ${WIN_X86_64:-<missing>}"

PUB_DATE=$(date -u +"%Y-%m-%dT%H:%M:%S.000Z")

# Assemble manifest incrementally so a missing platform degrades gracefully
# rather than producing invalid JSON. jq reads env vars we set here.
MANIFEST="$WORKDIR/latest.json"
jq -n \
  --arg version "$VERSION" \
  --arg pub_date "$PUB_DATE" \
  --arg notes "See https://github.com/$REPO/releases/tag/$TAG" \
  '{version: $version, notes: $notes, pub_date: $pub_date, platforms: {}}' \
  > "$MANIFEST"

add_platform() {
  local key="$1" name="$2"
  [ -z "$name" ] && return 0
  local sig url
  sig=$(read_sig "$name")
  url=$(asset_url "$name")
  jq --arg key "$key" --arg sig "$sig" --arg url "$url" \
    '.platforms[$key] = {signature: $sig, url: $url}' \
    "$MANIFEST" > "$MANIFEST.tmp"
  mv "$MANIFEST.tmp" "$MANIFEST"
  echo "[updater] + $key → $name"
}

add_platform "darwin-aarch64" "$MAC_AARCH64"
add_platform "darwin-x86_64"  "$MAC_X86_64"
add_platform "linux-x86_64"   "$LIN_X86_64"
add_platform "windows-x86_64" "$WIN_X86_64"

# Require every platform advertised by the public installers. A partial
# latest.json leaves install.sh resolving a documented platform to nothing.
missing_platforms=$(jq -r '
  ["darwin-aarch64", "darwin-x86_64", "linux-x86_64", "windows-x86_64"]
  - (.platforms | keys)
  | join(", ")
' "$MANIFEST")
if [ -n "$missing_platforms" ]; then
  echo "[updater] ERROR: missing required latest.json platform(s): $missing_platforms" >&2
  echo "[updater] Refusing to publish partial updater manifest." >&2
  exit 1
fi

echo "[updater] Final manifest:"
cat "$MANIFEST"

gh release upload "$TAG" "$MANIFEST" --repo "$REPO" --clobber
echo "[updater] Uploaded latest.json to $TAG"
