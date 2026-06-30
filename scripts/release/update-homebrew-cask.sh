#!/usr/bin/env bash
# Download macOS DMG release assets, compute SHA-256 checksums, render the
# Homebrew Cask template, and commit it to the tap repository.
#
# Usage:
#   update-homebrew-cask.sh <tag> <cask_template> <tap_dir>
#
# Example:
#   update-homebrew-cask.sh v0.54.0 packages/homebrew-cask/openhuman.rb /tmp/tap
#
# Required environment:
#   GITHUB_TOKEN — to download release assets
#
# The tap directory must be a git checkout of tinyhumansai/homebrew-openhuman.
set -euo pipefail

TAG="${1:?Usage: update-homebrew-cask.sh <tag> <cask_template> <tap_dir>}"
TEMPLATE="${2:?}"
TAP_DIR="${3:?}"
VERSION="${TAG#v}"
UPLOAD_REPO="${UPLOAD_REPO:-tinyhumansai/openhuman}"

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

echo "[homebrew-cask] Downloading macOS DMGs for $TAG ..."

SHA256_MACOS_ARM64=""
SHA256_MACOS_X64=""

for row in \
  "OpenHuman_${VERSION}_aarch64.dmg:SHA256_MACOS_ARM64" \
  "OpenHuman_${VERSION}_x64.dmg:SHA256_MACOS_X64"
do
  ASSET="${row%%:*}"
  VAR="${row##*:}"
  echo "[homebrew-cask]   Downloading $ASSET ..."
  gh release download "$TAG" \
    --pattern "$ASSET" \
    --repo "$UPLOAD_REPO" \
    --dir "$TMPDIR"
  SHA="$(openssl dgst -sha256 -r "$TMPDIR/$ASSET" | awk '{print $1}')"
  eval "${VAR}=${SHA}"
  echo "[homebrew-cask]   $ASSET -> $SHA"
done

mkdir -p "$TAP_DIR/Casks"

sed \
  -e "s/@VERSION@/${VERSION}/g" \
  -e "s/@SHA256_MACOS_ARM64@/${SHA256_MACOS_ARM64}/g" \
  -e "s/@SHA256_MACOS_X64@/${SHA256_MACOS_X64}/g" \
  "$TEMPLATE" > "$TAP_DIR/Casks/openhuman.rb"

echo "[homebrew-cask] Rendered cask -> $TAP_DIR/Casks/openhuman.rb"

cd "$TAP_DIR"
git config user.name  "${GIT_AUTHOR_NAME:-github-actions[bot]}"
git config user.email "${GIT_AUTHOR_EMAIL:-github-actions[bot]@users.noreply.github.com}"
git add Casks/openhuman.rb
if git diff --cached --quiet; then
  echo "[homebrew-cask] No changes to commit."
  exit 0
fi
git commit -m "chore: update cask to v${VERSION}"

if [[ "${DRY_RUN:-}" == "true" ]]; then
  echo "[homebrew-cask] DRY_RUN: skipping push"
else
  git push
  echo "[homebrew-cask] Pushed to tap"
fi
