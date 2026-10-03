#!/usr/bin/env bash
# Install the native modules the Rust coverage suite loads, and write their
# paths as KEY=VALUE lines to <env-file> for the checks that follow.
#
# Usage: install-test-modules.sh <dest-dir> <env-file>
#
# Release archives come from the module registry (test-module-assets.mjs),
# are cached by sha256 under $CI_CACHE_DIR/blobs when that is set, and are
# verified against the registry digest on every use, cached or not.
# libtinyconnectors has no published artifact, so it is built from the pinned
# submodule and cached by (submodule SHA, rustc version).
#
# tinybus refuses a module when any ancestor directory is owned by someone
# other than root or the current user, so <dest-dir> must sit under a tree the
# current user owns (the lane runner picks one per profile).
set -euo pipefail

cd "$(dirname "$0")/../../.."

log() { echo "[ci][test-modules] $*"; }

dest="${1:?usage: install-test-modules.sh <dest-dir> <env-file>}"
env_file="${2:?usage: install-test-modules.sh <dest-dir> <env-file>}"
host_key="${OH_TEST_MODULE_HOST_KEY:-ubuntu-22.04-x86_64}"
blobs="${CI_CACHE_DIR:+${CI_CACHE_DIR}/blobs}"
blobs="${blobs:-${dest}/.blobs}"
mkdir -p "${blobs}" "${dest}"

verify() { echo "$1  $2" | sha256sum --check --status; }

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
done < <(node scripts/ci/self-hosted/test-module-assets.mjs "${host_key}")

# The checkout action initializes root submodules, but the module build also
# needs tinyconnectors' nested tinybus dependency.
git submodule update --init --recursive vendor/tinyconnectors

conn_rev="$(git -C vendor/tinyconnectors rev-parse HEAD)"
rustc_rev="$(rustc -V | sha256sum | cut -c1-12)"
conn_cached="${blobs}/tinyconnectors-${conn_rev}-${rustc_rev}.so"
if [ -f "${conn_cached}" ]; then
  log "tinyconnectors: cache hit ${conn_rev:0:12}"
else
  conn_target="${TINYCONNECTORS_TARGET_DIR:-vendor/tinyconnectors/target}"
  log "tinyconnectors: building ${conn_rev:0:12} into ${conn_target}"
  bash scripts/ci-cancel-aware.sh cargo build --release \
    --manifest-path vendor/tinyconnectors/crates/tinyconnectors/Cargo.toml \
    --target-dir "${conn_target}"
  cp "${conn_target}/release/libtinyconnectors.so" "${conn_cached}.part"
  mv "${conn_cached}.part" "${conn_cached}"
fi
touch "${conn_cached}"
mkdir -p "${dest}/tinyconnectors"
cp "${conn_cached}" "${dest}/tinyconnectors/libtinyconnectors.so"

{
  echo "TINYJUICE_TEST_MODULE=${dest}/tinyjuice/libtinyjuice_module.so"
  echo "TINYSEARCH_TEST_MODULE=${dest}/tinysearch/libtinysearch.so"
  echo "TINYCONNECTORS_TEST_MODULE=${dest}/tinyconnectors/libtinyconnectors.so"
} > "${env_file}"
log "wrote ${env_file}"
