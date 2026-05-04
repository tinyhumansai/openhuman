#!/usr/bin/env bash
# validate-release-assets.sh — check that a release advertises every
# supported platform on both the GitHub release assets and the Tauri
# updater manifest (`latest.json`).
#
# Regression guard for tinyhumansai/openhuman#785: when a release ships
# without a Linux asset, `install.sh` on Linux falls through every
# resolver and prints a confusing failure. This script catches the drift
# at the source so maintainers notice before users do.
#
# Usage:
#   scripts/validate-release-assets.sh <release.json> <latest.json>
#
# Inputs:
#   release.json — raw body from GET /repos/:owner/:repo/releases/<id>
#                  (or /releases/latest).
#   latest.json  — the updater manifest uploaded as a release asset.
#
# Exit codes:
#   0 — every supported platform has a matching asset + latest.json entry.
#   1 — at least one platform is missing from either source (details on stderr).
#   2 — bad arguments or invalid JSON.
#
# Example (local):
#   gh api repos/tinyhumansai/openhuman/releases/latest > /tmp/release.json
#   curl -fsSL https://github.com/tinyhumansai/openhuman/releases/latest/download/latest.json > /tmp/latest.json
#   scripts/validate-release-assets.sh /tmp/release.json /tmp/latest.json

set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "Usage: $0 <release.json> <latest.json>" >&2
  exit 2
fi

release_json="$1"
latest_json="$2"

for f in "${release_json}" "${latest_json}"; do
  if [ ! -s "${f}" ]; then
    echo "validate-release-assets: missing or empty file: ${f}" >&2
    exit 2
  fi
done

if ! command -v python3 >/dev/null 2>&1; then
  echo "validate-release-assets: python3 is required" >&2
  exit 2
fi

python3 - "${release_json}" "${latest_json}" <<'PY'
import json, re, sys

# Platforms that install.sh / install.ps1 claim to support. Keep in sync
# with scripts/install.sh (OS/arch case branches) and the Tauri updater
# manifest consumers in app/src-tauri/tauri.conf.json.
SUPPORTED = [
    "darwin-aarch64",
    "darwin-x86_64",
    "linux-x86_64",
    "windows-x86_64",
]

# Release asset name patterns per platform. Mirrors the patterns used in
# release.yml's "Validate required installer assets exist" step and the
# regex branches inside install.sh's resolve_from_release_api.
ASSET_PATTERNS = {
    "darwin-aarch64": r"aarch64.*\.app\.tar\.gz$|aarch64\.dmg$",
    "darwin-x86_64":  r"(x86_64-apple-darwin|x64).*\.app\.tar\.gz$|x64\.dmg$",
    "linux-x86_64":   r"\.AppImage$",
    "windows-x86_64": r"x64.*\.msi$|x64.*setup\.exe$",
}

release_path, latest_path = sys.argv[1], sys.argv[2]
try:
    release = json.load(open(release_path))
    latest  = json.load(open(latest_path))
except json.JSONDecodeError as e:
    print(f"validate-release-assets: invalid JSON: {e}", file=sys.stderr)
    sys.exit(2)

asset_names = [a.get("name", "") for a in release.get("assets", [])]
latest_platforms = latest.get("platforms", {}) or {}

# Mirror scripts/install.sh's fallback chain: accept the bare platform key
# OR a `-appimage` / `-app` suffixed variant, matching what the Tauri
# updater manifest may emit. Without this the validator false-flags a
# correctly-shipped release that uses the suffix form.
def _has_platform(key):
    return key in latest_platforms or f"{key}-appimage" in latest_platforms or f"{key}-app" in latest_platforms

missing_latest = [p for p in SUPPORTED if not _has_platform(p)]
missing_assets = [
    p for p in SUPPORTED
    if not any(re.search(ASSET_PATTERNS[p], n) for n in asset_names)
]

tag = release.get("tag_name") or release.get("name") or "<unknown tag>"
if missing_latest or missing_assets:
    print(f"Release validation FAILED for {tag}", file=sys.stderr)
    if missing_latest:
        print(f"  Missing from latest.json: {', '.join(missing_latest)}", file=sys.stderr)
    if missing_assets:
        print(f"  Missing release assets:   {', '.join(missing_assets)}", file=sys.stderr)
    print(
        "  See scripts/install.sh for the supported-platform matrix.",
        file=sys.stderr,
    )
    sys.exit(1)

print(f"Release validation passed for {tag}. Supported: {', '.join(SUPPORTED)}")
PY
