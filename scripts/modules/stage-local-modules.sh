#!/usr/bin/env bash
# Helpers the local Rust test runners (scripts/test-rust-with-mock.sh,
# scripts/test-rust-e2e.sh) source to stage native modules.
#
# Released modules (the default, same as CI): `stage_release_module <id>` downloads
# the registry-pinned archive, verifies its sha256 and puts it on
# OPENHUMAN_MODULE_PATH. Source-built modules: with OPENHUMAN_MODULE_SOURCE=source,
# `stage_source_modules <id>...` builds them from the vendored submodules with
# scripts/modules/build.mjs.
#
# Needs REPO_ROOT, module_ext and wallet_platform (or module_platform) set by the caller.

local_module_source_is_source() {
  [ "${OPENHUMAN_MODULE_SOURCE:-}" = "source" ]
}

# stage_release_module <id> <host-platform> <ext>; prints the staged directory.
stage_release_module() {
  local id="$1" platform="$2" ext="$3"
  local asset url filename sha dir archive
  asset="$(node "$REPO_ROOT/scripts/ci/self-hosted/test-module-assets.mjs" "$platform" "$id")"
  IFS=$'\t' read -r _id url filename sha <<< "$asset"
  dir="$REPO_ROOT/target/test-modules/$id/$sha"
  archive="$dir/$filename"
  if ! compgen -G "$dir/*.$ext" >/dev/null; then
    echo "Downloading the checksum-pinned ${id} test module ..." >&2
    mkdir -p "$dir"
    # Callers run this in $(...), where errexit does not apply: a failed
    # download or digest check must stop here, never fall through to tar.
    curl --fail --location --silent --show-error "$url" --output "$archive" || return 1
    if command -v shasum >/dev/null 2>&1; then
      echo "${sha}  $archive" | shasum -a 256 -c >&2 || { rm -f "$archive"; return 1; }
    else
      echo "${sha}  $archive" | sha256sum --check >&2 || { rm -f "$archive"; return 1; }
    fi
    tar --no-same-owner -xzf "$archive" -C "$dir"
  fi
  printf '%s\n' "$dir"
}

# stage_release_artifact <id> <host-platform> <ext>; prints the staged module
# library itself, for tests that load a module by file rather than through
# OPENHUMAN_MODULE_PATH.
stage_release_artifact() {
  local id="$1" dir
  dir="$(stage_release_module "$id" "$2" "$3")" || return 1
  if [ "$3" = "dll" ]; then
    printf '%s\n' "$dir/${id}_module.dll"
  else
    printf '%s\n' "$dir/lib${id}_module.$3"
  fi
}

# stage_source_modules <id>... : build from the pinned submodules into the
# default out dir (target/module-sources/<id>/). The runners put the directories
# they want on OPENHUMAN_MODULE_PATH themselves, so a module is visible only to
# the processes meant to see it. build.mjs prints its exports; they are dropped
# here on purpose (OPENHUMAN_MODULE_SOURCE_<ID> would switch a module on for
# every suite).
stage_source_modules() {
  node "$REPO_ROOT/scripts/modules/build.mjs" "$@" >/dev/null
}
