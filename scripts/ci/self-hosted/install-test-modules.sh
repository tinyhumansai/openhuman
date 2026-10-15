#!/usr/bin/env bash
# Install the native modules the Rust coverage suite loads, and write their
# paths as KEY=VALUE lines to <env-file> for the checks that follow.
#
# Usage: install-test-modules.sh <dest-dir> <env-file>
#
# Release archives come from the module registry (test-module-assets.mjs),
# are cached by sha256 under $CI_CACHE_DIR/blobs when that is set, and are
# verified against the registry digest on every use, cached or not.
# All nine modules, tinyconnectors included, come from releases; nothing is
# compiled from source here (#7618).
#
# tinybus refuses a module when any ancestor directory is owned by someone
# other than root or the current user, so <dest-dir> must sit under a tree the
# current user owns (the lane runner picks one per profile).
set -euo pipefail

cd "$(dirname "$0")/../../.."

log() { echo "[ci][test-modules] $*"; }

dest="${1:?usage: install-test-modules.sh <dest-dir> <env-file>}"
env_file="${2:?usage: install-test-modules.sh <dest-dir> <env-file>}"
host_key="${OH_MODULE_HOST_KEY:-ubuntu-22.04-x86_64}"
blobs="${CI_CACHE_DIR:+${CI_CACHE_DIR}/blobs}"
blobs="${blobs:-${dest}/.blobs}"
mkdir -p "${blobs}" "${dest}"

verify() { echo "$1  $2" | sha256sum --check --status; }

# Resolve before starting work: process substitution hides the resolver's exit
# status, which used to continue with missing fixtures after a registry error.
assets="$(node scripts/ci/self-hosted/test-module-assets.mjs "${host_key}")"
if [ -z "${assets}" ]; then
  echo "::error::registry resolved no native test module assets" >&2
  exit 1
fi

while IFS=$'\t' read -r id url archive sha; do
  cached="${blobs}/${sha}-${archive}"
  if [ -f "${cached}" ] && verify "${sha}" "${cached}"; then
    log "${id}: cache hit ${archive}"
  else
    log "${id}: downloading ${archive}"
    curl --fail --location --silent --show-error --retry 4 --retry-delay 2 \
      "${url}" --output "${cached}.part"
    if ! verify "${sha}" "${cached}.part"; then
      rm -f "${cached}.part"
      echo "::error::${archive} does not match the registry sha256 ${sha}" >&2
      exit 1
    fi
    mv "${cached}.part" "${cached}"
  fi
  touch "${cached}"
  rm -rf "${dest:?}/${id}"
  mkdir -p "${dest}/${id}"
  # Do not restore the release builder's uid into the trusted tree.
  tar --no-same-owner -xzf "${cached}" -C "${dest}/${id}"
done <<< "${assets}"

# Every staged directory goes on the search path the loader already honours
# (release mode); each holds the released library.
module_dirs=""
for id in tinybox tinydocs tinywallet tinychannels tinymcp tinyjuice tinysearch tinyhosts tinyconnectors tinysecurity; do
  module_dirs="${module_dirs:+${module_dirs}:}${dest}/${id}"
done
# mcp_registry_multi_server loads TinyMCP by file, not by search path, as
# scripts/test-rust-with-mock.sh supplies it locally.
tinymcp_artifact="$(find "${dest}/tinymcp" -maxdepth 2 -type f \
  \( -name libtinymcp_module.so -o -name libtinymcp_module.dylib -o -name tinymcp_module.dll \) | head -n 1)"
if [ -z "${tinymcp_artifact}" ]; then
  echo "::error::the tinymcp release holds no module library" >&2
  exit 1
fi
{
  echo "OPENHUMAN_MODULE_PATH=${module_dirs}${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}"
  echo "OPENHUMAN_TINYMCP_TEST_MODULE=${tinymcp_artifact}"
} > "${env_file}"
log "wrote ${env_file}"
