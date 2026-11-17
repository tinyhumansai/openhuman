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

# Resolve the exact repositories the gate inspects, including shared sources
# and providers, from its authoritative inventory. Keep inventory failures fatal.
modules="$(node --input-type=module -e '
  import { modulePinRepositories } from "./scripts/lib/module-pin-inventory.mjs";
  console.log(modulePinRepositories().join("\n"));
')"
while IFS= read -r module; do
  [ -n "$module" ] || continue
  if [ "$(git -C "$module" rev-parse --is-shallow-repository)" = true ]; then
    git -C "$module" fetch --quiet --no-recurse-submodules --unshallow --tags origin
  else
    git -C "$module" fetch --quiet --no-recurse-submodules --tags origin
  fi
done <<< "$modules"
echo "[ci][submodule-tags] fetched tags for module submodules"
