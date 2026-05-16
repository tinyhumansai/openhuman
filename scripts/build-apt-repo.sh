#!/usr/bin/env bash
# Build a signed Debian apt repository from one or more .deb files.
# Requires: dpkg-dev (dpkg-scanpackages), apt-utils (apt-ftparchive), gzip, gpg, python3
#
# Usage:
#   build-apt-repo.sh <output_dir> <pkg1.deb> [<pkg2.deb> ...]
#
# The GPG signing key must be imported into the agent before calling.
# Set APT_SIGNING_KEY_ID to select the key; leave unset to use the default.
set -euo pipefail

OUTPUT_DIR="$1"; shift
DEB_FILES=("$@")

echo "[apt-repo] Building repository at $OUTPUT_DIR"

# ── Pool ───────────────────────────────────────────────────────────────────────
mkdir -p "$OUTPUT_DIR/pool/main"
for deb in "${DEB_FILES[@]}"; do
  cp "$deb" "$OUTPUT_DIR/pool/main/"
  echo "[apt-repo]   + pool/main/$(basename "$deb")"
done

# ── Per-architecture Packages files ───────────────────────────────────────────
FILTER_PY="$(mktemp --suffix=.py)"
trap 'rm -f "$FILTER_PY"' EXIT

cat > "$FILTER_PY" << 'PYEOF'
import sys, re

arch = sys.argv[1]
data = open(sys.argv[2]).read()
out = []
for block in data.strip().split('\n\n'):
    if re.search(r'^Architecture:\s+' + re.escape(arch) + r'\s*$', block, re.MULTILINE):
        out.append(block.rstrip())
if out:
    print('\n\n'.join(out) + '\n')
PYEOF

ALL_PACKAGES="$(mktemp)"
(cd "$OUTPUT_DIR" && dpkg-scanpackages --multiversion pool/main 2>/dev/null) > "$ALL_PACKAGES"

for arch in amd64 arm64; do
  dir="$OUTPUT_DIR/dists/stable/main/binary-${arch}"
  mkdir -p "$dir"
  python3 "$FILTER_PY" "$arch" "$ALL_PACKAGES" > "$dir/Packages"
  gzip -9c "$dir/Packages" > "$dir/Packages.gz"
  lines=$(wc -l < "$dir/Packages")
  echo "[apt-repo]   binary-${arch}/Packages: ${lines} lines"
done
rm -f "$ALL_PACKAGES"

# ── Release file ───────────────────────────────────────────────────────────────
RELEASE_CONF="$(mktemp)"
cat > "$RELEASE_CONF" << 'EOF'
APT::FTPArchive::Release::Origin "OpenHuman";
APT::FTPArchive::Release::Label "OpenHuman";
APT::FTPArchive::Release::Suite "stable";
APT::FTPArchive::Release::Codename "stable";
APT::FTPArchive::Release::Architectures "amd64 arm64";
APT::FTPArchive::Release::Components "main";
APT::FTPArchive::Release::Description "OpenHuman official apt repository";
EOF

(cd "$OUTPUT_DIR" && apt-ftparchive -c "$RELEASE_CONF" release dists/stable) \
  > "$OUTPUT_DIR/dists/stable/Release"
rm -f "$RELEASE_CONF"
echo "[apt-repo]   Release generated"

# ── Sign ───────────────────────────────────────────────────────────────────────
GPG_ARGS=(--batch --yes)
[[ -n "${APT_SIGNING_KEY_ID:-}" ]] && GPG_ARGS+=(--local-user "$APT_SIGNING_KEY_ID")

gpg "${GPG_ARGS[@]}" --clearsign \
  -o "$OUTPUT_DIR/dists/stable/InRelease" \
  "$OUTPUT_DIR/dists/stable/Release"

gpg "${GPG_ARGS[@]}" -abs \
  -o "$OUTPUT_DIR/dists/stable/Release.gpg" \
  "$OUTPUT_DIR/dists/stable/Release"

echo "[apt-repo]   Release signed"

# ── Export public key ─────────────────────────────────────────────────────────
EXPORT_ARGS=(--batch --yes --armor --export)
[[ -n "${APT_SIGNING_KEY_ID:-}" ]] && EXPORT_ARGS+=("$APT_SIGNING_KEY_ID")
gpg "${EXPORT_ARGS[@]}" > "$OUTPUT_DIR/KEY.gpg"
echo "[apt-repo]   Public key → KEY.gpg"

echo "[apt-repo] Done. Files:"
find "$OUTPUT_DIR" -type f | sort | sed 's|^|  |'
