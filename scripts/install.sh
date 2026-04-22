#!/usr/bin/env bash
# OpenHuman Installer (macOS/Linux)
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash

set -euo pipefail

INSTALLER_VERSION="1.0.0"
REPO="tinyhumansai/openhuman"
LATEST_JSON_URL="https://github.com/${REPO}/releases/latest/download/latest.json"
LATEST_RELEASE_API_URL="https://api.github.com/repos/${REPO}/releases/latest"

CHANNEL="stable"
DRY_RUN=false
VERBOSE=false

if [ -t 1 ]; then
  RED='\033[0;31m'
  GREEN='\033[0;32m'
  YELLOW='\033[0;33m'
  CYAN='\033[0;36m'
  NC='\033[0m'
else
  RED=''
  GREEN=''
  YELLOW=''
  CYAN=''
  NC=''
fi

log_info() { echo -e "${CYAN}→${NC} $*"; }
log_ok() { echo -e "${GREEN}✓${NC} $*"; }
log_warn() { echo -e "${YELLOW}!${NC} $*"; }
log_err() { echo -e "${RED}x${NC} $*" >&2; }

usage() {
  cat <<'EOF'
OpenHuman Installer

Usage: install.sh [OPTIONS]

Options:
  --help            Show help
  --version         Show installer version
  --channel VALUE   Release channel (default: stable)
  --dry-run         Print actions without mutating local files
  --verbose         Enable verbose output

Examples:
  curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash
  curl -fsSL ... | bash -s -- --dry-run
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --help|-h)
      usage
      exit 0
      ;;
    --version)
      echo "openhuman-installer ${INSTALLER_VERSION}"
      exit 0
      ;;
    --channel)
      CHANNEL="${2:-}"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=true
      shift
      ;;
    --verbose)
      VERBOSE=true
      shift
      ;;
    *)
      log_err "Unknown option: $1"
      usage
      exit 1
      ;;
  esac
done

if [ "${CHANNEL}" != "stable" ]; then
  log_err "Only --channel stable is currently supported."
  exit 1
fi

for cmd in curl mktemp tar; do
  if ! command -v "${cmd}" >/dev/null 2>&1; then
    log_err "Missing required command: ${cmd}"
    exit 1
  fi
done

OS_RAW="$(uname -s)"
ARCH_RAW="$(uname -m)"
OS=""
ARCH=""
PLATFORM_KEY=""

case "${OS_RAW}" in
  Darwin) OS="darwin" ;;
  Linux) OS="linux" ;;
  CYGWIN*|MINGW*|MSYS*)
    log_err "Windows detected. Use PowerShell installer:"
    echo "  irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex"
    exit 1
    ;;
  *)
    log_err "Unsupported OS: ${OS_RAW}"
    exit 1
    ;;
esac

case "${ARCH_RAW}" in
  x86_64|amd64) ARCH="x86_64" ;;
  arm64|aarch64) ARCH="aarch64" ;;
  *)
    log_err "Unsupported architecture: ${ARCH_RAW}"
    exit 1
    ;;
esac

if [ "${OS}" = "linux" ] && [ "${ARCH}" != "x86_64" ]; then
  log_err "Linux installer currently supports x86_64 only."
  exit 1
fi

if [ "${OS}" = "darwin" ] && [ "${ARCH}" = "aarch64" ]; then
  PLATFORM_KEY="darwin-aarch64"
elif [ "${OS}" = "darwin" ] && [ "${ARCH}" = "x86_64" ]; then
  PLATFORM_KEY="darwin-x86_64"
elif [ "${OS}" = "linux" ] && [ "${ARCH}" = "x86_64" ]; then
  PLATFORM_KEY="linux-x86_64"
fi

log_ok "Detected platform: ${OS}/${ARCH}"

TMP_DIR="$(mktemp -d)"
cleanup() {
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

LATEST_JSON_PATH="${TMP_DIR}/latest.json"
RELEASE_JSON_PATH="${TMP_DIR}/release.json"

LATEST_VERSION=""
ASSET_URL=""
ASSET_NAME=""
ASSET_SHA256=""

resolve_from_latest_json() {
  if ! curl -fsSL "${LATEST_JSON_URL}" -o "${LATEST_JSON_PATH}"; then
    return 1
  fi

  if ! command -v python3 >/dev/null 2>&1; then
    log_warn "python3 is not available; cannot parse latest.json reliably."
    return 1
  fi

  local parsed
  parsed="$(python3 - "${LATEST_JSON_PATH}" "${PLATFORM_KEY}" <<'PY'
import json, sys
path, key = sys.argv[1], sys.argv[2]
with open(path, "r", encoding="utf-8") as f:
    data = json.load(f)
version = data.get("version", "")
platforms = data.get("platforms", {})
entry = platforms.get(key) or platforms.get(f"{key}-appimage") or platforms.get(f"{key}-app")
url = ""
if isinstance(entry, dict):
    url = entry.get("url", "")
print(version)
print(url)
PY
)" || return 1

  LATEST_VERSION="$(echo "${parsed}" | sed -n '1p')"
  ASSET_URL="$(echo "${parsed}" | sed -n '2p')"
  ASSET_NAME="$(basename "${ASSET_URL}")"

  [ -n "${ASSET_URL}" ]
}

resolve_from_release_api() {
  if ! curl -fsSL "${LATEST_RELEASE_API_URL}" -o "${RELEASE_JSON_PATH}"; then
    return 1
  fi

  if ! command -v python3 >/dev/null 2>&1; then
    log_warn "python3 is not available; cannot parse release API fallback."
    return 1
  fi

  local parsed
  parsed="$(python3 - "${RELEASE_JSON_PATH}" "${OS}" "${ARCH}" <<'PY'
import json, re, sys
path, os_name, arch = sys.argv[1], sys.argv[2], sys.argv[3]
with open(path, "r", encoding="utf-8") as f:
    data = json.load(f)
tag = (data.get("tag_name") or "").lstrip("v")
assets = data.get("assets", [])

def choose_asset():
    names = [a.get("name", "") for a in assets]
    chosen = None
    if os_name == "darwin" and arch == "aarch64":
        for n in names:
            if re.search(r"aarch64.*\\.app\\.tar\\.gz$", n):
                chosen = n
                break
        if not chosen:
            for n in names:
                if re.search(r"aarch64\\.dmg$", n):
                    chosen = n
                    break
    elif os_name == "darwin" and arch == "x86_64":
        for n in names:
            if re.search(r"(x86_64-apple-darwin|x64).*\\.app\\.tar\\.gz$", n):
                chosen = n
                break
        if not chosen:
            for n in names:
                if re.search(r"x64\\.dmg$", n):
                    chosen = n
                    break
    elif os_name == "linux" and arch == "x86_64":
        for n in names:
            if n.endswith(".AppImage"):
                chosen = n
                break
    if not chosen:
        return "", "", ""
    for asset in assets:
        if asset.get("name") == chosen:
            return chosen, asset.get("browser_download_url", ""), (asset.get("digest", "") or "").replace("sha256:", "")
    return "", "", ""

name, url, digest = choose_asset()
print(tag)
print(name)
print(url)
print(digest)
PY
)" || return 1

  if [ -z "${LATEST_VERSION}" ]; then
    LATEST_VERSION="$(echo "${parsed}" | sed -n '1p')"
  fi
  ASSET_NAME="$(echo "${parsed}" | sed -n '2p')"
  ASSET_URL="$(echo "${parsed}" | sed -n '3p')"
  ASSET_SHA256="$(echo "${parsed}" | sed -n '4p')"

  # Exit 0 on success, 2 when API responded but no compatible asset was found.
  # Callers can distinguish "no asset" (2) from network/parse errors (1).
  if [ -n "${ASSET_URL}" ]; then
    return 0
  fi
  return 2
}

resolve_release_digest() {
  if [ -z "${ASSET_NAME}" ]; then
    return 0
  fi
  if [ ! -s "${RELEASE_JSON_PATH}" ]; then
    if ! curl -fsSL "${LATEST_RELEASE_API_URL}" -o "${RELEASE_JSON_PATH}"; then
      return 0
    fi
  fi
  if ! command -v python3 >/dev/null 2>&1; then
    return 0
  fi
  local digest
  digest="$(python3 - "${RELEASE_JSON_PATH}" "${ASSET_NAME}" <<'PY'
import json, sys
path, name = sys.argv[1], sys.argv[2]
with open(path, "r", encoding="utf-8") as f:
    data = json.load(f)
for asset in data.get("assets", []):
    if asset.get("name") == name:
        d = asset.get("digest", "") or ""
        print(d.replace("sha256:", ""))
        break
PY
)"
  if [ -n "${digest}" ]; then
    ASSET_SHA256="${digest}"
  fi
}

if resolve_from_latest_json; then
  log_ok "Resolved latest release via latest.json (${LATEST_VERSION})"
else
  log_warn "latest.json lookup failed. Falling back to releases API."
  resolve_from_release_api
  resolve_rc=$?
  if [ "${resolve_rc}" -ne 0 ]; then
    if [ "${OS}" = "linux" ] && [ "${DRY_RUN}" = true ] && [ "${resolve_rc}" -eq 2 ]; then
      log_warn "No Linux release asset is currently published. Dry-run will skip install steps."
      echo "DRY RUN: no compatible asset available for ${OS}/${ARCH}"
      exit 0
    fi
    log_err "Could not resolve a compatible asset for ${OS}/${ARCH}."
    exit 1
  fi
  log_ok "Resolved latest release via releases API (${LATEST_VERSION})"
fi

resolve_release_digest

if [ -z "${ASSET_URL}" ]; then
  log_err "Could not determine download URL for ${OS}/${ARCH}."
  exit 1
fi

DOWNLOAD_PATH="${TMP_DIR}/${ASSET_NAME}"
log_info "Downloading ${ASSET_NAME}"
if [ "${DRY_RUN}" = true ]; then
  echo "DRY RUN: curl -fL ${ASSET_URL} -o ${DOWNLOAD_PATH}"
else
  curl -fL "${ASSET_URL}" -o "${DOWNLOAD_PATH}"
fi

compute_sha256() {
  local file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${file}" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "${file}" | awk '{print $1}'
  elif command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 "${file}" | awk '{print $2}'
  else
    return 1
  fi
}

if [ -n "${ASSET_SHA256}" ]; then
  if [ "${DRY_RUN}" = true ]; then
    echo "DRY RUN: verify sha256 ${ASSET_SHA256} for ${DOWNLOAD_PATH}"
  else
    actual_sha256="$(compute_sha256 "${DOWNLOAD_PATH}" || true)"
    if [ -z "${actual_sha256}" ]; then
      log_warn "No checksum command available; skipping digest verification."
    elif [ "${actual_sha256}" != "${ASSET_SHA256}" ]; then
      log_err "SHA256 mismatch for ${ASSET_NAME}"
      log_err "Expected: ${ASSET_SHA256}"
      log_err "Actual:   ${actual_sha256}"
      exit 1
    else
      log_ok "Integrity verified (sha256)"
    fi
  fi
else
  log_warn "No SHA256 digest available for ${ASSET_NAME}; skipping integrity verification."
fi

ensure_local_bin_path() {
  local bin_dir="${HOME}/.local/bin"
  if echo ":${PATH}:" | grep -q ":${bin_dir}:"; then
    return 0
  fi
  local shell_name config_file
  shell_name="$(basename "${SHELL:-/bin/bash}")"
  case "${shell_name}" in
    zsh) config_file="${HOME}/.zshrc" ;;
    bash) config_file="${HOME}/.bashrc" ;;
    *) config_file="${HOME}/.profile" ;;
  esac

  if [ "${DRY_RUN}" = true ]; then
    echo "DRY RUN: ensure ${bin_dir} in PATH via ${config_file}"
    return 0
  fi

  if [ ! -f "${config_file}" ]; then
    touch "${config_file}"
  fi
  if ! grep -q '.local/bin' "${config_file}"; then
    {
      echo ""
      echo '# OpenHuman installer - ensure local user binaries are on PATH'
      echo 'export PATH="$HOME/.local/bin:$PATH"'
    } >> "${config_file}"
    log_ok "Added ~/.local/bin to ${config_file}"
  fi
}

install_macos() {
  local apps_dir="${HOME}/Applications"
  local app_path="${apps_dir}/OpenHuman.app"
  mkdir -p "${apps_dir}"

  if [[ "${ASSET_NAME}" == *.app.tar.gz ]]; then
    log_info "Installing OpenHuman.app into ${apps_dir}"
    if [ "${DRY_RUN}" = true ]; then
      echo "DRY RUN: tar -xzf ${DOWNLOAD_PATH} -C ${TMP_DIR}"
      echo "DRY RUN: replace ${app_path}"
    else
      tar -xzf "${DOWNLOAD_PATH}" -C "${TMP_DIR}"
      if [ ! -d "${TMP_DIR}/OpenHuman.app" ]; then
        log_err "Archive did not contain OpenHuman.app"
        exit 1
      fi
      rm -rf "${app_path}"
      cp -R "${TMP_DIR}/OpenHuman.app" "${app_path}"
    fi
  elif [[ "${ASSET_NAME}" == *.dmg ]]; then
    log_info "Mounting DMG and copying OpenHuman.app"
    if [ "${DRY_RUN}" = true ]; then
      echo "DRY RUN: hdiutil attach ${DOWNLOAD_PATH}"
      echo "DRY RUN: copy OpenHuman.app to ${app_path}"
    else
      if ! command -v hdiutil >/dev/null 2>&1; then
        log_err "hdiutil not available, cannot install from DMG."
        exit 1
      fi
      mount_output="$(hdiutil attach "${DOWNLOAD_PATH}" -nobrowse)"
      mount_point="$(echo "${mount_output}" | awk '/\/Volumes\// {print $NF; exit}')"
      if [ -z "${mount_point}" ] || [ ! -d "${mount_point}/OpenHuman.app" ]; then
        log_err "Could not find OpenHuman.app in mounted DMG."
        echo "${mount_output}"
        exit 1
      fi
      rm -rf "${app_path}"
      cp -R "${mount_point}/OpenHuman.app" "${app_path}"
      hdiutil detach "${mount_point}" >/dev/null
    fi
  else
    log_err "Unsupported macOS asset type: ${ASSET_NAME}"
    exit 1
  fi

  log_ok "Installed at ${app_path}"
  echo ""
  echo "OpenHuman is ready."
  echo "Launch: open \"${app_path}\""
  echo "Uninstall: rm -rf \"${app_path}\""
}

install_linux() {
  local bin_dir="${HOME}/.local/bin"
  local app_path="${bin_dir}/openhuman"
  local desktop_dir="${HOME}/.local/share/applications"
  local desktop_file="${desktop_dir}/openhuman.desktop"

  mkdir -p "${bin_dir}" "${desktop_dir}"

  if [[ "${ASSET_NAME}" != *.AppImage ]]; then
    log_err "Expected AppImage for Linux install, got: ${ASSET_NAME}"
    exit 1
  fi

  if [ "${DRY_RUN}" = true ]; then
    echo "DRY RUN: install ${DOWNLOAD_PATH} -> ${app_path}"
  else
    cp "${DOWNLOAD_PATH}" "${app_path}"
    chmod +x "${app_path}"
  fi

  ensure_local_bin_path

  if [ "${DRY_RUN}" = true ]; then
    echo "DRY RUN: write ${desktop_file}"
  else
    cat > "${desktop_file}" <<EOF
[Desktop Entry]
Type=Application
Name=OpenHuman
Comment=OpenHuman desktop assistant
Exec=${app_path}
TryExec=${app_path}
Terminal=false
Categories=Utility;
EOF
  fi

  log_ok "Installed binary at ${app_path}"
  echo ""
  echo "OpenHuman is ready."
  echo "Launch: ${app_path}"
  echo "Uninstall: rm -f \"${app_path}\" \"${desktop_file}\""
}

if [ "${OS}" = "darwin" ]; then
  install_macos
else
  install_linux
fi
