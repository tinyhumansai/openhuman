#!/usr/bin/env bash
# scripts/dev-setup-linker.sh
#
# Opt in to a faster local linker (mold on Linux, lld on macOS) WITHOUT
# touching the repo's tracked .cargo/config.toml. That file intentionally
# ships the linker block commented out (see the comment header there) — an
# unconditional `linker=`/`-fuse-ld=` would hard-fail every build on a
# machine that doesn't have the linker installed, including the
# `pnpm rust:check` pre-push hook.
#
# This script instead writes (or merges into) $CARGO_HOME/config.toml
# (defaulting to ~/.cargo/config.toml), which Cargo layers on top of any
# repo-level config. That keeps the opt-in per-machine and out of git.
#
# Usage:
#   scripts/dev-setup-linker.sh            # detect + install the config
#   scripts/dev-setup-linker.sh --dry-run  # print what would change, no writes
#   scripts/dev-setup-linker.sh -n         # same as --dry-run
#
# Idempotent: running it twice leaves the config file unchanged the second
# time (it detects its own previously-written block and skips it).

set -euo pipefail

DRY_RUN=0
for arg in "$@"; do
  case "$arg" in
    --dry-run | -n)
      DRY_RUN=1
      ;;
    -h | --help)
      sed -n '2,25p' "$0"
      exit 0
      ;;
    *)
      echo "dev-setup-linker.sh: unknown argument: $arg" >&2
      exit 1
      ;;
  esac
done

log() {
  echo "[dev-setup-linker] $*"
}

CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"
CONFIG_PATH="$CARGO_HOME_DIR/config.toml"

# Marker so re-runs can detect (and skip) a block this script already wrote.
MARKER_BEGIN="# BEGIN openhuman dev-setup-linker (scripts/dev-setup-linker.sh)"
MARKER_END="# END openhuman dev-setup-linker"

uname_s="$(uname -s)"
uname_m="$(uname -m)"

target=""
rustflags_value=""
tool_name=""

case "$uname_s" in
  Linux)
    if ! command -v mold >/dev/null 2>&1; then
      log "mold not found on PATH. Install it first:"
      log "  apt: sudo apt-get install mold"
      log "  dnf: sudo dnf install mold"
      log "  pacman: sudo pacman -S mold"
      exit 1
    fi
    tool_name="mold"
    case "$uname_m" in
      x86_64 | amd64)
        target="x86_64-unknown-linux-gnu"
        ;;
      aarch64 | arm64)
        target="aarch64-unknown-linux-gnu"
        ;;
      *)
        log "unsupported Linux architecture: $uname_m"
        exit 1
        ;;
    esac
    rustflags_value='["-C", "link-arg=-fuse-ld=mold"]'
    ;;
  Darwin)
    # Prefer lld from a Homebrew LLVM install (ld64.lld). Recent Xcode ships
    # Apple's ld-prime, which is already fast, so this is often only a
    # marginal win on Apple Silicon — measure before relying on it (see
    # .cargo/config.toml's comment header).
    lld_path=""
    for candidate in \
      /opt/homebrew/opt/llvm/bin/ld64.lld \
      /usr/local/opt/llvm/bin/ld64.lld; do
      if [ -x "$candidate" ]; then
        lld_path="$candidate"
        break
      fi
    done
    if [ -z "$lld_path" ]; then
      log "ld64.lld not found. Install LLVM first: brew install llvm"
      exit 1
    fi
    tool_name="lld ($lld_path)"
    case "$uname_m" in
      arm64)
        target="aarch64-apple-darwin"
        ;;
      x86_64)
        target="x86_64-apple-darwin"
        ;;
      *)
        log "unsupported macOS architecture: $uname_m"
        exit 1
        ;;
    esac
    rustflags_value="[\"-C\", \"link-arg=-fuse-ld=$lld_path\"]"
    ;;
  *)
    log "unsupported OS: $uname_s (only Linux and macOS are supported)"
    exit 1
    ;;
esac

log "detected $tool_name for target $target"

block=$(
  cat <<EOF
$MARKER_BEGIN
[target.$target]
rustflags = $rustflags_value
$MARKER_END
EOF
)

if [ -f "$CONFIG_PATH" ] && grep -Fq "$MARKER_BEGIN" "$CONFIG_PATH"; then
  log "$CONFIG_PATH already has an openhuman dev-setup-linker block — nothing to do"
  log "(delete the block between the BEGIN/END markers and re-run to refresh it)"
  exit 0
fi

# A TOML table cannot be declared twice. Do not append a second target table
# to a developer's global Cargo config: merging their existing rustflags could
# silently discard linker options they deliberately configured.
if [ -f "$CONFIG_PATH" ] && awk -v table="[target.$target]" '
  {
    sub(/[[:space:]]*#.*/, "")
    gsub(/^[[:space:]]+|[[:space:]]+$/, "")
    if ($0 == table) found = 1
  }
  END { exit !found }
' "$CONFIG_PATH"; then
  log "$CONFIG_PATH already declares [target.$target]; refusing to modify it"
  log "add the linker rustflags manually or remove that table and re-run"
  exit 1
fi

if [ "$DRY_RUN" -eq 1 ]; then
  log "--dry-run: would write to $CONFIG_PATH:"
  echo ""
  echo "$block"
  exit 0
fi

mkdir -p "$CARGO_HOME_DIR"

if [ -f "$CONFIG_PATH" ]; then
  log "appending linker block to existing $CONFIG_PATH"
  {
    echo ""
    echo "$block"
  } >>"$CONFIG_PATH"
else
  log "creating $CONFIG_PATH"
  echo "$block" >"$CONFIG_PATH"
fi

log "done. New cargo invocations will use $tool_name for target $target."
