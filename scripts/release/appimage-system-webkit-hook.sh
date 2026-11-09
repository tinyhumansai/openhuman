# shellcheck shell=bash
# linuxdeploy AppRun hook: prefer the host's WebKitGTK over the bundled one (#7652).
#
# strip-appimage-graphics-libs.sh installs this as apprun-hooks/openhuman-system-webkit.sh,
# stamps the bundled library's real name below, and makes AppRun source it before
# every other hook. AppRun provides `$this_dir` (the AppDir) and the positional args.
#
# The bundled WebKitGTK comes from the build runner, but the bundled Mesa/EGL is
# stripped so the host's graphics drivers are used. A host Mesa much newer than the
# bundled WebKitWebProcess can make it abort with "Could not create default EGL
# display: EGL_BAD_PARAMETER" (Fedora 44, Mesa 26.2). When the host has its own
# webkit2gtk-4.1 at least as new as the bundled one, run the app against the host's
# library directory instead, the same as the reporter's working
# `LD_LIBRARY_PATH=/usr/lib64` workaround. The host WebKit then launches its own
# helper processes, built against the host's Mesa. Bundled libraries still cover
# anything the host lacks through the binary's $ORIGIN RUNPATH.
#
# OPENHUMAN_APPIMAGE_WEBKIT=bundled keeps the bundled WebKit.

OPENHUMAN_BUNDLED_WEBKIT='@BUNDLED_WEBKIT@'

# Prints the host directory holding libwebkit2gtk-4.1.so.0 for this machine's
# architecture, from the loader cache.
openhuman_host_webkit_dir() {
  local arch ldconfig lib=""
  case "$(uname -m)" in
    x86_64) arch='x86-64' ;;
    aarch64) arch='AArch64' ;;
    *) return 1 ;;
  esac
  # Debian-family PATHs for normal users omit /sbin.
  for ldconfig in ldconfig /sbin/ldconfig /usr/sbin/ldconfig; do
    lib="$("$ldconfig" -p 2>/dev/null \
      | awk -v arch="$arch" '$1 == "libwebkit2gtk-4.1.so.0" && index($0, arch) { print $NF; exit }')" || lib=""
    [ -n "$lib" ] && break
  done
  [ -n "$lib" ] || return 1
  dirname "$lib"
}

# True when the host's webkit2gtk-4.1 is usable and not older than the bundled one.
# ponytail: compares libtool real names (libwebkit2gtk-4.1.so.0.X.Y), which grow
# with every WebKitGTK release that adds API; an unreadable name keeps the bundle.
openhuman_prefer_host_webkit() {
  local dir="$1" host oldest
  [ -e "$dir/libjavascriptcoregtk-4.1.so.0" ] || return 1
  host="$(basename "$(readlink -f "$dir/libwebkit2gtk-4.1.so.0")")" || return 1
  case "$host" in libwebkit2gtk-4.1.so.0.*) ;; *) return 1 ;; esac
  oldest="$(printf '%s\n%s\n' "$OPENHUMAN_BUNDLED_WEBKIT" "$host" | sort -V 2>/dev/null | head -n 1)" || return 1
  [ "$oldest" = "$OPENHUMAN_BUNDLED_WEBKIT" ]
}

if [ "${OPENHUMAN_APPIMAGE_WEBKIT:-}" != "bundled" ] \
  && openhuman_host_webkit_dir_value="$(openhuman_host_webkit_dir)" \
  && openhuman_prefer_host_webkit "$openhuman_host_webkit_dir_value"; then
  echo "[openhuman] using host WebKitGTK from $openhuman_host_webkit_dir_value (set OPENHUMAN_APPIMAGE_WEBKIT=bundled to use the bundled copy)" >&2
  export LD_LIBRARY_PATH="$openhuman_host_webkit_dir_value${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  # The linuxdeploy GTK hook forces X11 for the bundled stack; keep that default.
  export GDK_BACKEND="${GDK_BACKEND:-x11}"
  exec "$this_dir/usr/bin/OpenHuman" "$@"
fi
unset openhuman_host_webkit_dir_value
