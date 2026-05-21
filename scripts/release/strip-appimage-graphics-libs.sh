#!/usr/bin/env bash
# Strip host graphics libraries from AppImage bundles so they load the user's
# system Mesa/libdrm/libva at launch instead of the older versions baked in by
# lib4bin's ldd-walk on the ubuntu-22.04 build runner.
#
# Without this, AppImages built on Mesa 22.x fail to initialize on systems
# with newer GPUs (RDNA3, Intel Arc, Lovelace) because the bundled drivers
# can't talk to the host kernel/driver stack. AppImage convention is to never
# ship graphics drivers — they must come from the host. See:
# https://github.com/AppImageCommunity/pkg2appimage/blob/master/excludelist
#
# Only top-level lib directories are swept. CEF's own subdirs (swiftshader/,
# locales/, libcef.so neighbors) are left alone — CEF ships its own
# GLES/EGL implementation that must stay bundled.
#
# Usage: strip-appimage-graphics-libs.sh <bundle-root> [bundle-root...]
#   where <bundle-root> contains an `appimage/` subdir with *.AppImage files.
#
# Env:
#   TAURI_SIGNING_PRIVATE_KEY            — re-sign modified artifacts when set
#   TAURI_SIGNING_PRIVATE_KEY_PASSWORD   — passphrase for the key (may be empty)
#   APPIMAGETOOL_URL                     — override appimagetool download URL
#   APPIMAGETOOL_SHA256                  — expected SHA256 of the download
#                                          (verified before use when set; rotate
#                                          alongside APPIMAGETOOL_URL)

set -euo pipefail

EXCLUDE_PATTERNS=(
  'libGL.so.*'
  'libGLX.so.*'
  'libGLdispatch.so.*'
  'libGLESv1_CM.so.*'
  'libGLESv2.so.*'
  'libEGL.so.*'
  'libgbm.so.*'
  'libdrm.so.*'
  'libdrm_*.so.*'
  'libva.so.*'
  'libva-drm.so.*'
  'libva-glx.so.*'
  'libva-x11.so.*'
  'libvdpau.so.*'
  'libxcb-dri2.so.*'
  'libxcb-dri3.so.*'
  'libxcb-glx.so.*'
  'libxcb-present.so.*'
)

# Default to a pinned release tag rather than the mutable `continuous` asset so
# CI builds are reproducible and resistant to upstream replacement. Override via
# APPIMAGETOOL_URL (and bump APPIMAGETOOL_SHA256 alongside it).
APPIMAGETOOL_URL="${APPIMAGETOOL_URL:-https://github.com/AppImage/appimagetool/releases/download/1.9.0/appimagetool-x86_64.AppImage}"
APPIMAGETOOL_SHA256="${APPIMAGETOOL_SHA256:-}"

ensure_appimagetool() {
  if command -v appimagetool >/dev/null 2>&1; then
    APPIMAGETOOL_BIN="$(command -v appimagetool)"
    return
  fi
  local tool=/tmp/appimagetool.AppImage
  if [ ! -x "$tool" ]; then
    echo "[strip-libs] Downloading appimagetool from $APPIMAGETOOL_URL"
    curl -fsSL "$APPIMAGETOOL_URL" -o "$tool"
    if [ -n "$APPIMAGETOOL_SHA256" ]; then
      echo "[strip-libs] Verifying appimagetool sha256"
      if ! echo "${APPIMAGETOOL_SHA256}  ${tool}" | sha256sum -c -; then
        echo "[strip-libs] ERROR: appimagetool sha256 mismatch — refusing to run" >&2
        rm -f "$tool"
        exit 1
      fi
    else
      echo "[strip-libs] WARNING: APPIMAGETOOL_SHA256 not set — skipping integrity check" >&2
    fi
    chmod +x "$tool"
  fi
  APPIMAGETOOL_BIN="$tool"
}

appimage_loader_name() {
  local target_arch="${APPIMAGE_TARGET_ARCH:-${MATRIX_TARGET:-$(uname -m)}}"
  case "$target_arch" in
    x86_64*|amd64*)
      echo "ld-linux-x86-64.so.2"
      ;;
    *)
      return 1
      ;;
  esac
}

host_dynamic_loader() {
  local loader_name="$1"
  local candidates=()
  case "$loader_name" in
    ld-linux-x86-64.so.2)
      candidates=(
        "/lib64/$loader_name"
        "/lib/x86_64-linux-gnu/$loader_name"
        "/usr/lib64/$loader_name"
        "/usr/lib/$loader_name"
      )
      ;;
    ld-linux-aarch64.so.1)
      candidates=(
        "/lib/$loader_name"
        "/lib/aarch64-linux-gnu/$loader_name"
        "/usr/lib/aarch64-linux-gnu/$loader_name"
      )
      ;;
  esac

  local candidate
  for candidate in "${candidates[@]}"; do
    if [ -f "$candidate" ]; then
      echo "$candidate"
      return 0
    fi
  done
  return 1
}

is_executable_elf() {
  local candidate
  candidate="$1"
  [ -f "$candidate" ] || return 1
  [ -x "$candidate" ] || return 1
  [ "$(LC_ALL=C head -c 4 "$candidate" 2>/dev/null || true)" = $'\177ELF' ]
}

emit_entry_if_elf() {
  local candidate="$1"
  if is_executable_elf "$candidate"; then
    printf '%s\0' "$candidate"
  fi
}

emit_desktop_exec_candidate() {
  local appdir="$1"
  local command="$2"
  local candidate

  [ -n "$command" ] || return 0
  case "$command" in
    /*)
      emit_entry_if_elf "$appdir$command"
      ;;
    */*)
      emit_entry_if_elf "$appdir/$command"
      ;;
    *)
      for candidate in "$appdir/$command" "$appdir/bin/$command" "$appdir/usr/bin/$command"; do
        emit_entry_if_elf "$candidate"
      done
      ;;
  esac
}

discover_appimage_entry_binaries() {
  local appdir="$1"
  local desktop line exec_line command root candidate

  emit_entry_if_elf "$appdir/AppRun"
  emit_entry_if_elf "$appdir/sharun"

  while IFS= read -r -d '' desktop; do
    while IFS= read -r line || [ -n "$line" ]; do
      case "$line" in
        Exec=*)
          exec_line="${line#Exec=}"
          case "$exec_line" in
            \"*\")
              command="${exec_line#\"}"
              command="${command%%\"*}"
              ;;
            \'*\')
              command="${exec_line#\'}"
              command="${command%%\'*}"
              ;;
            *)
              command="${exec_line%%[[:space:]]*}"
              ;;
          esac
          emit_desktop_exec_candidate "$appdir" "$command"
          ;;
      esac
    done < "$desktop"
  done < <(find "$appdir" -maxdepth 1 -type f -name '*.desktop' -print0)

  for root in "$appdir" "$appdir/bin" "$appdir/usr/bin"; do
    [ -d "$root" ] || continue
    while IFS= read -r -d '' candidate; do
      emit_entry_if_elf "$candidate"
    done < <(find "$root" -maxdepth 1 -type f -perm /111 -print0)
  done
}

uses_sharun_launcher() {
  local appdir="$1"
  local candidate
  while IFS= read -r -d '' candidate; do
    if grep -a -q "Interpreter not found!" "$candidate" 2>/dev/null; then
      return 0
    fi
  done < <(discover_appimage_entry_binaries "$appdir")
  return 1
}

ensure_sharun_interpreter() {
  local appdir="$1"
  if ! uses_sharun_launcher "$appdir"; then
    return 1
  fi

  local loader_name
  if ! loader_name="$(appimage_loader_name)"; then
    echo "[strip-libs] ERROR: AppImage uses sharun but architecture is unsupported; cannot determine required loader" >&2
    exit 1
  fi

  local target="$appdir/lib/$loader_name"
  if [ -e "$target" ]; then
    return 1
  fi

  local source
  if ! source="$(host_dynamic_loader "$loader_name")"; then
    echo "[strip-libs] ERROR: AppImage uses sharun but host loader $loader_name was not found; refusing to ship an AppImage that exits with 'Interpreter not found!'" >&2
    exit 1
  fi

  mkdir -p "$appdir/lib"
  cp -L "$source" "$target"
  chmod 755 "$target"
  echo "[strip-libs]   bundling sharun interpreter ${target#"$appdir"/} from $source"
  return 0
}

validate_sharun_lib_path() {
  local appdir="$1"
  if ! uses_sharun_launcher "$appdir"; then
    return 0
  fi

  local lib_path="$appdir/shared/lib/lib.path"
  if [ ! -s "$lib_path" ]; then
    echo "[strip-libs] ERROR: sharun AppImage is missing shared/lib/lib.path; refusing to ship an AppImage that exits with 'Interpreter not found!'" >&2
    exit 1
  fi

  if grep -E '(^|[+:])/home/runner/|(^|[+:])/__w/' "$lib_path" >/dev/null; then
    echo "[strip-libs] ERROR: shared/lib/lib.path contains CI runner paths; regenerate it with bundle-relative entries before release." >&2
    exit 1
  fi
}

strip_one_appimage() {
  local img="$1"
  local original
  original="$(realpath "$img")"
  local name
  name="$(basename "$original")"
  local workdir
  workdir="$(mktemp -d)"

  echo "[strip-libs] Processing $original"
  (
    cd "$workdir"
    chmod +x "$original"
    if ! "$original" --appimage-extract >/dev/null; then
      echo "[strip-libs] ERROR: --appimage-extract failed for $original" >&2
      exit 1
    fi
  )

  local appdir="$workdir/squashfs-root"
  local removed=0
  local added_loader=0
  local lib_roots=()
  for candidate in \
    "$appdir/usr/lib" \
    "$appdir/usr/lib/x86_64-linux-gnu" \
    "$appdir/shared/lib" \
    "$appdir/shared/lib/x86_64-linux-gnu" \
    "$appdir/lib" \
    "$appdir/lib/x86_64-linux-gnu"; do
    [ -d "$candidate" ] && lib_roots+=("$candidate")
  done

  if [ "${#lib_roots[@]}" -eq 0 ]; then
    echo "[strip-libs] WARNING: no known lib roots inside $original — layout changed?" >&2
  else
    for root in "${lib_roots[@]}"; do
      for pattern in "${EXCLUDE_PATTERNS[@]}"; do
        while IFS= read -r -d '' f; do
          echo "[strip-libs]   removing ${f#"$appdir"/}"
          rm -f "$f"
          removed=$((removed + 1))
        done < <(find "$root" -maxdepth 1 -name "$pattern" -print0)
      done
    done
  fi

  if ensure_sharun_interpreter "$appdir"; then
    added_loader=1
  fi
  validate_sharun_lib_path "$appdir"

  if [ "$removed" -eq 0 ] && [ "$added_loader" -eq 0 ]; then
    echo "[strip-libs] No graphics libs or missing sharun interpreter found in $original; leaving unchanged."
    rm -rf "$workdir"
    return
  fi
  echo "[strip-libs] Removed $removed file(s), added $added_loader loader file(s); repacking AppImage."

  local rebuilt="$workdir/$name"
  (
    cd "$workdir"
    ARCH=x86_64 "$APPIMAGETOOL_BIN" --appimage-extract-and-run \
      --no-appstream squashfs-root "$rebuilt" >/dev/null
  )
  mv "$rebuilt" "$original"
  rm -rf "$workdir"
  MODIFIED_PATHS+=("$original")
}

resign_artifact() {
  local file="$1"
  if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
    return
  fi
  if ! command -v cargo-tauri >/dev/null 2>&1; then
    echo "[strip-libs] WARNING: cargo-tauri not on PATH; cannot re-sign $file" >&2
    return
  fi
  echo "[strip-libs] Re-signing $file"
  rm -f "$file.sig"
  cargo tauri signer sign \
    --private-key "$TAURI_SIGNING_PRIVATE_KEY" \
    --password "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" \
    "$file" >/dev/null
}

main() {
  if [ $# -lt 1 ]; then
    echo "Usage: $0 <bundle-root> [bundle-root...]" >&2
    exit 2
  fi
  ensure_appimagetool
  shopt -s nullglob
  MODIFIED_PATHS=()
  local found_any=0
  for root in "$@"; do
    [ -d "$root/appimage" ] || continue
    for img in "$root/appimage"/*.AppImage; do
      found_any=1
      strip_one_appimage "$img"
    done
  done
  if [ "$found_any" -eq 0 ]; then
    echo "[strip-libs] No AppImages found under any provided bundle root." >&2
    return
  fi

  # Re-sign each modified .AppImage and rebuild its updater tarball + sig.
  # The updater tarball is just a gzipped tar of the .AppImage (Tauri convention),
  # so its contents are stale the moment we mutate the AppImage.
  for original in "${MODIFIED_PATHS[@]:-}"; do
    [ -n "$original" ] || continue
    resign_artifact "$original"

    local tar="$original.tar.gz"
    if [ -e "$tar" ]; then
      echo "[strip-libs] Rebuilding $(basename "$tar")"
      tar -C "$(dirname "$original")" -czf "$tar" "$(basename "$original")"
      resign_artifact "$tar"
    fi
  done
}

main "$@"
