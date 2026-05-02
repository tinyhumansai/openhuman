#!/usr/bin/env bash
# =============================================================================
# upload_sentry_symbols.sh
#
# Uploads Rust debug symbols and source maps to Sentry for the Tauri app.
# This enables proper stack trace symbolication in Sentry for production builds.
#
# Usage:
#   ./scripts/upload_sentry_symbols.sh [version]
#
# Environment variables required:
#   SENTRY_AUTH_TOKEN  - Sentry authentication token (required)
#   SENTRY_ORG         - Sentry organization slug (required)
#   SENTRY_PROJECT     - Sentry project name (required)
#
# Optional environment variables:
#   SENTRY_VERSION     - Release version (defaults to: openhuman@{version})
#   DEBUG_SYMBOLS_PATH - Path to debug symbols (defaults to: target/release/deps)
# =============================================================================

set -euo pipefail

# Color output helpers
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Validate required environment variables
check_env_vars() {
    local missing_vars=()

    if [[ -z "${SENTRY_AUTH_TOKEN:-}" ]]; then
        missing_vars+=("SENTRY_AUTH_TOKEN")
    fi

    if [[ -z "${SENTRY_ORG:-}" ]]; then
        missing_vars+=("SENTRY_ORG")
    fi

    if [[ -z "${SENTRY_PROJECT:-}" ]]; then
        missing_vars+=("SENTRY_PROJECT")
    fi

    if [[ ${#missing_vars[@]} -gt 0 ]]; then
        log_error "Missing required environment variables: ${missing_vars[*]}"
        log_error "Please set these variables before running this script."
        exit 1
    fi
}

# Detect or install sentry-cli
ensure_sentry_cli() {
    if command -v sentry-cli &> /dev/null; then
        log_info "sentry-cli already installed: $(sentry-cli --version)"
        return 0
    fi

    log_info "Installing sentry-cli..."

    # Detect OS and architecture. The release-asset suffix matches what
    # `getsentry/sentry-cli` actually publishes — OS segment is
    # title-cased (Linux/Darwin/Windows), not lowercase. Lowercase 404s
    # silently and we end up writing GitHub's HTML error page to a file
    # the script then tries to execute ("Not: command not found").
    local os_arch
    case "$(uname -s)" in
        Linux*)
            case "$(uname -m)" in
                x86_64|amd64)
                    os_arch="Linux-x86_64"
                    ;;
                aarch64|arm64)
                    os_arch="Linux-aarch64"
                    ;;
                *)
                    log_error "Unsupported architecture: $(uname -m)"
                    exit 1
                    ;;
            esac
            ;;
        Darwin*)
            # The mac build is published as a universal binary, not
            # per-arch, so both Intel and Apple Silicon use the same
            # asset — there is no Darwin-x86_64 / Darwin-arm64.
            os_arch="Darwin-universal"
            ;;
        MINGW*|CYGWIN*|MSYS*)
            os_arch="Windows-x86_64.exe"
            ;;
        *)
            log_error "Unsupported operating system: $(uname -s)"
            exit 1
            ;;
    esac

    local version="2.34.2"
    local download_url="https://github.com/getsentry/sentry-cli/releases/download/${version}/sentry-cli-${os_arch}"

    # Create temporary directory for installation. Cleanup runs as a
    # RETURN trap (function-scoped) rather than EXIT so it can still see
    # `tmp_dir` — EXIT fires after the function returns, by which point
    # `local tmp_dir` is out of scope and `set -u` errors with
    # "tmp_dir: unbound variable".
    local tmp_dir
    tmp_dir="$(mktemp -d)"
    trap 'rm -rf "$tmp_dir"' RETURN

    # Download and install. `--fail` / `--fail-with-body` is critical:
    # without it, curl returns 0 on a 404 and writes the error HTML to
    # the destination file. Same for wget without `--content-on-error`.
    log_info "Downloading sentry-cli ${version} for ${os_arch}..."
    if command -v curl &> /dev/null; then
        curl --fail --silent --show-error --location "${download_url}" -o "${tmp_dir}/sentry-cli" || {
            log_error "Failed to download sentry-cli from ${download_url}"
            exit 1
        }
    elif command -v wget &> /dev/null; then
        wget --quiet --show-progress=off "${download_url}" -O "${tmp_dir}/sentry-cli" || {
            log_error "Failed to download sentry-cli from ${download_url}"
            exit 1
        }
    else
        log_error "Neither curl nor wget found. Cannot download sentry-cli."
        exit 1
    fi

    # Validate the downloaded file is actually an executable, not an HTML
    # error page that slipped through (defence-in-depth alongside --fail).
    if [[ ! -s "${tmp_dir}/sentry-cli" ]]; then
        log_error "sentry-cli download is empty"
        exit 1
    fi
    if head -c 4 "${tmp_dir}/sentry-cli" | grep -q '^<'; then
        log_error "sentry-cli download looks like HTML (got an error page from ${download_url})"
        exit 1
    fi

    # Make executable and install to ~/.cargo/bin or /usr/local/bin
    chmod +x "${tmp_dir}/sentry-cli"

    local install_dir="${HOME}/.cargo/bin"
    mkdir -p "${install_dir}"

    if [[ -w "${install_dir}" ]]; then
        mv "${tmp_dir}/sentry-cli" "${install_dir}/sentry-cli"
        log_info "sentry-cli installed to ${install_dir}/sentry-cli"
    else
        # Fallback to /usr/local/bin (may require sudo)
        if sudo mv "${tmp_dir}/sentry-cli" "/usr/local/bin/sentry-cli" 2>/dev/null; then
            log_info "sentry-cli installed to /usr/local/bin/sentry-cli"
        else
            log_error "Cannot write to ${install_dir} or /usr/local/bin. Please install sentry-cli manually."
            exit 1
        fi
    fi

    # Update PATH hash for current session (won't persist without shell restart)
    hash -r
}

# Upload debug symbols to Sentry
upload_symbols() {
    local version="${1:-}"
    local symbols_path="${2:-target/release/deps}"

    if [[ -z "${version}" ]]; then
        log_error "Version is required"
        exit 1
    fi

    # Honor SENTRY_RELEASE if set so DIFs attach to the same release name
    # the running binaries report (`openhuman@<version>+<sha>`). Without this,
    # CI uploads to `openhuman@<version>` while events are tagged
    # `openhuman@<version>+<sha>` — a different release, so Sentry never
    # joins frames to symbols and stack traces stay un-symbolicated.
    # Falls back to the bare-version tag for local invocations that don't
    # set SENTRY_RELEASE.
    local release_name="${SENTRY_RELEASE:-openhuman@${version}}"

    log_info "Uploading Rust debug symbols for release: ${release_name}"
    log_info "Symbols path: ${symbols_path}"

    # Create Sentry release
    log_info "Creating/updating Sentry release..."
    sentry-cli releases new "${release_name}" || true
    # Use --ignore-missing for shallow clones or CI environments
    sentry-cli releases set-commits --auto --ignore-missing "${release_name}" || true

    # Upload debug symbols + source bundles. `--include-sources` makes
    # `sentry-cli` package the referenced source files into a `.src.zip`
    # alongside the DIF, so Sentry renders surrounding source lines in
    # Rust stack traces instead of bare `function + 0xNNN`. CI runs from a
    # full workspace checkout, so the source paths embedded in the DWARF
    # resolve and the bundle is built correctly.
    log_info "Uploading debug symbols..."
    local upload_args=(
        "upload-dif"
        "--org" "${SENTRY_ORG}"
        "--project" "${SENTRY_PROJECT}"
        "--include-sources"
        # sentry-cli 3.x renamed `warning` → `warn`. Use the short form;
        # `warning` is rejected as `invalid value '...' for '--log-level'`
        # on 3.x and the script silently skips uploads.
        "--log-level=warn"
    )

    # Find and upload all debug symbol files
    if [[ -d "${symbols_path}" ]]; then
        # Upload .dwp (dwarf packages), .debug files, and .pdb files
        # Debug symbols are indexed by debug-ID, not release-scoped
        log_info "Scanning for debug symbols in ${symbols_path}..."
        sentry-cli "${upload_args[@]}" "${symbols_path}" || {
            log_warn "Some debug symbols may have failed to upload"
        }
    else
        log_warn "Symbols path does not exist: ${symbols_path}"
        log_info "Looking for any release artifacts..."
    fi

    # Finalize the release
    log_info "Finalizing release..."
    sentry-cli releases finalize "${release_name}"

    log_info "Successfully uploaded symbols for ${release_name}"
}

# Main execution
main() {
    log_info "=== Sentry Symbol Upload Script ==="

    # Parse arguments
    local version="${1:-}"
    local symbols_path="${2:-}"

    # Check environment variables
    check_env_vars

    # Ensure sentry-cli is available
    ensure_sentry_cli

    # Validate version argument
    if [[ -z "${version}" ]]; then
        # Try to extract version from Cargo.toml or package.json
        if [[ -f "app/src-tauri/Cargo.toml" ]]; then
            version=$(grep -m1 '^version\s*=' app/src-tauri/Cargo.toml | sed 's/version\s*=\s*"\([^"]*\)"/\1/')
            log_info "Detected version from Cargo.toml: ${version}"
        elif [[ -f "app/package.json" ]]; then
            version=$(grep -m1 '"version"' app/package.json | sed 's/.*"version": *"\([^"]*\)".*/\1/')
            log_info "Detected version from package.json: ${version}"
        else
            log_error "Could not determine version. Please provide it as an argument."
            log_info "Usage: $0 <version> [symbols_path]"
            exit 1
        fi
    fi

    # Default symbols path if not provided
    if [[ -z "${symbols_path}" ]]; then
        symbols_path="target/release/deps"
    fi

    # Upload symbols
    upload_symbols "${version}" "${symbols_path}"

    log_info "=== Upload complete ==="
}

# Run main function
main "$@"
