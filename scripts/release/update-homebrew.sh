#!/usr/bin/env bash
# Download release tarballs, compute SHA-256 checksums, render the Homebrew
# formula from the template and commit it to the tap repository.
#
# Usage:
#   update-homebrew.sh <tag> <formula_template> <tap_dir>
#
# Example:
#   update-homebrew.sh v0.5.0 packages/homebrew/openhuman.rb /tmp/tap
#
# Required environment:
#   GITHUB_TOKEN — to download release assets
#
# The tap directory must be a git checkout of tinyhumansai/homebrew-openhuman.
set -euo pipefail

TAG="${1:?Usage: update-homebrew.sh <tag> <formula_template> <tap_dir>}"
TEMPLATE="${2:?}"
TAP_DIR="${3:?}"
VERSION="${TAG#v}"
UPLOAD_REPO="${UPLOAD_REPO:-tinyhumansai/openhuman}"

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

echo "[homebrew] Downloading release tarballs for $TAG ..."

SHA256_MACOS_ARM64=""
SHA256_MACOS_X64=""
SHA256_LINUX_X64=""
SHA256_LINUX_ARM64=""

for row in \
  "aarch64-apple-darwin:SHA256_MACOS_ARM64" \
  "x86_64-apple-darwin:SHA256_MACOS_X64" \
  "x86_64-unknown-linux-gnu:SHA256_LINUX_X64" \
  "aarch64-unknown-linux-gnu:SHA256_LINUX_ARM64"
do
  TARGET="${row%%:*}"
  VAR="${row##*:}"
  TARBALL="openhuman-core-${VERSION}-${TARGET}.tar.gz"
  echo "[homebrew]   Downloading $TARBALL ..."
  gh release download "$TAG" \
    --pattern "$TARBALL" \
    --repo "$UPLOAD_REPO" \
    --dir "$TMPDIR"
  SHA=$(openssl dgst -sha256 -r "$TMPDIR/$TARBALL" | awk '{print $1}')
  eval "${VAR}=${SHA}"
  echo "[homebrew]   $TARGET → $SHA"
done

# ── Render formula ───────────────────────────────────────────────────────────
mkdir -p "$TAP_DIR/Formula"

sed \
  -e "s/@VERSION@/${VERSION}/g" \
  -e "s/@SHA256_MACOS_ARM64@/${SHA256_MACOS_ARM64}/g" \
  -e "s/@SHA256_MACOS_X64@/${SHA256_MACOS_X64}/g" \
  -e "s/@SHA256_LINUX_X64@/${SHA256_LINUX_X64}/g" \
  -e "s/@SHA256_LINUX_ARM64@/${SHA256_LINUX_ARM64}/g" \
  "$TEMPLATE" > "$TAP_DIR/Formula/openhuman.rb"

echo "[homebrew] Rendered formula → $TAP_DIR/Formula/openhuman.rb"

# ── Commit and push ──────────────────────────────────────────────────────────
cd "$TAP_DIR"
git config user.name  "${GIT_AUTHOR_NAME:-github-actions[bot]}"
git config user.email "${GIT_AUTHOR_EMAIL:-github-actions[bot]@users.noreply.github.com}"
git add Formula/openhuman.rb
if git diff --cached --quiet; then
  echo "[homebrew] No changes to commit."
  exit 0
fi
git commit -m "chore: update formula to v${VERSION}"

if [[ "${DRY_RUN:-}" == "true" ]]; then
  echo "[homebrew] DRY_RUN: skipping push"
else
  git push
  echo "[homebrew] Pushed to tap"
fi
