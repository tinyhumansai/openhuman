#!/usr/bin/env bash
# Download Linux CLI tarballs from a GitHub release, build .deb packages,
# then build a signed apt repository and optionally deploy to gh-pages.
#
# Usage:
#   build-apt-packages.sh <tag> [--deploy-gh-pages <gh_pages_dir>]
#
# Required environment:
#   GITHUB_TOKEN         — download release assets
#   APT_SIGNING_KEY_ID   — GPG key ID for signing (must be imported)
#
# Optional environment:
#   UPLOAD_REPO          — GitHub repo slug (default: tinyhumansai/openhuman)
#   DRY_RUN              — set to "true" to skip git push
set -euo pipefail

TAG="${1:?Usage: build-apt-packages.sh <tag> [--deploy-gh-pages <gh_pages_dir>]}"
shift
VERSION="${TAG#v}"
UPLOAD_REPO="${UPLOAD_REPO:-tinyhumansai/openhuman}"

DEPLOY_GH_PAGES=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --deploy-gh-pages) DEPLOY_GH_PAGES="${2:?--deploy-gh-pages requires a path}"; shift 2 ;;
    *) echo "Unknown arg: $1"; exit 1 ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

# ── Download tarballs ────────────────────────────────────────────────────────
echo "[apt] Downloading Linux CLI tarballs for $TAG ..."
mkdir -p "$TMPDIR/tarballs" "$TMPDIR/bins"

for target in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
  TARBALL="openhuman-core-${VERSION}-${target}.tar.gz"
  gh release download "$TAG" \
    --pattern "$TARBALL" \
    --repo "$UPLOAD_REPO" \
    --dir "$TMPDIR/tarballs"
  echo "[apt]   Downloaded $TARBALL"
done

# ── Extract binaries ─────────────────────────────────────────────────────────
tar -xzf "$TMPDIR/tarballs/openhuman-core-${VERSION}-x86_64-unknown-linux-gnu.tar.gz" \
  -C "$TMPDIR/bins"
mv "$TMPDIR/bins/openhuman-core" "$TMPDIR/bins/openhuman-core-amd64"

tar -xzf "$TMPDIR/tarballs/openhuman-core-${VERSION}-aarch64-unknown-linux-gnu.tar.gz" \
  -C "$TMPDIR/bins"
mv "$TMPDIR/bins/openhuman-core" "$TMPDIR/bins/openhuman-core-arm64"

chmod +x "$TMPDIR/bins/openhuman-core-amd64" "$TMPDIR/bins/openhuman-core-arm64"

# ── Build .deb packages ─────────────────────────────────────────────────────
echo "[apt] Building .deb packages ..."
bash "$REPO_ROOT/packages/deb/build.sh" "$TMPDIR/bins/openhuman-core-amd64" "${VERSION}" amd64
bash "$REPO_ROOT/packages/deb/build.sh" "$TMPDIR/bins/openhuman-core-arm64" "${VERSION}" arm64

ls -lh openhuman_*.deb

# ── Build apt repository ────────────────────────────────────────────────────
APT_REPO_DIR="$TMPDIR/apt-repo"
echo "[apt] Building apt repository ..."
bash "$REPO_ROOT/scripts/build-apt-repo.sh" "$APT_REPO_DIR" \
  "openhuman_${VERSION}_amd64.deb" \
  "openhuman_${VERSION}_arm64.deb"

# ── Deploy to gh-pages ───────────────────────────────────────────────────────
if [[ -n "$DEPLOY_GH_PAGES" ]]; then
  echo "[apt] Deploying apt repo to gh-pages at $DEPLOY_GH_PAGES ..."
  mkdir -p "$DEPLOY_GH_PAGES/apt"
  rm -rf "$DEPLOY_GH_PAGES/apt/"*
  cp -r "$APT_REPO_DIR/." "$DEPLOY_GH_PAGES/apt/"

  cd "$DEPLOY_GH_PAGES"
  git config user.name  "${GIT_AUTHOR_NAME:-github-actions[bot]}"
  git config user.email "${GIT_AUTHOR_EMAIL:-github-actions[bot]@users.noreply.github.com}"
  git add apt/
  if git diff --cached --quiet; then
    echo "[apt] No changes."
    exit 0
  fi
  git commit -m "chore(apt): publish v${VERSION}"

  if [[ "${DRY_RUN:-}" == "true" ]]; then
    echo "[apt] DRY_RUN: skipping push"
  else
    git push origin gh-pages
    echo "[apt] Pushed to gh-pages"
  fi
fi
