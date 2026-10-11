#!/usr/bin/env bash
# Container jobs mount a runner-owned checkout. Trust only that checkout and
# its initialized submodules, retaining Git's ownership checks everywhere else.
set -euo pipefail
checkout_root="$(cd "${1:-${GITHUB_WORKSPACE:-.}}" && pwd -P)"
git config --global --add safe.directory "$checkout_root"
git -C "$checkout_root" submodule foreach --recursive \
  'git config --global --add safe.directory "$PWD"'
