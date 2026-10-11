#!/usr/bin/env bash
# Fetch tag refs (and full history when shallow) for every module submodule.
#
# actions/checkout populates the pinned submodule commits but not their tags.
# scripts/ci/check-module-pins.mjs uses `git describe --tags` to compare each
# source contract with its registry artifact version, so the tags must exist.
# Called by ci-lite.yml's module-pin gate and by the lane runner. Only these
# repositories' own tags are needed, so nested submodules are not fetched.
set -euo pipefail

cd "$(dirname "$0")/../.."

for module in \
  vendor/tinydocs vendor/tinywallet vendor/tinymemory vendor/tinyjuice \
  vendor/tinyvoice vendor/tinymcp vendor/tinyconnectors \
  vendor/tinysearch vendor/tinybox vendor/tinychannels vendor/tinyhosts
do
  if [ "$(git -C "$module" rev-parse --is-shallow-repository)" = true ]; then
    git -C "$module" fetch --quiet --no-recurse-submodules --unshallow --tags origin
  else
    git -C "$module" fetch --quiet --no-recurse-submodules --tags origin
  fi
done
echo "[ci][submodule-tags] fetched tags for module submodules"
