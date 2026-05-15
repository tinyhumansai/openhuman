#!/usr/bin/env bash
# ws-reset: hard-reset the workspace to upstream/main and refresh submodules.
#
# Fetches the `upstream` remote (tinyhumansai/openhuman), checks out `main`,
# hard-resets it to `upstream/main`, and recursively updates submodules.
#
# Bails out if the working tree has uncommitted changes (use --force to override).

set -euo pipefail

FORCE=0
for arg in "$@"; do
  case "$arg" in
    -f|--force) FORCE=1 ;;
    -h|--help)
      sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "ws-reset: unknown arg: $arg" >&2
      exit 2
      ;;
  esac
done

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "ws-reset: not inside a git repo" >&2
  exit 1
fi

if ! git remote get-url upstream >/dev/null 2>&1; then
  echo "ws-reset: no 'upstream' remote configured" >&2
  exit 1
fi

if [ "$FORCE" -ne 1 ] && [ -n "$(git status --porcelain --untracked-files=all)" ]; then
  echo "ws-reset: working tree has uncommitted changes or untracked files. Re-run with --force to discard." >&2
  exit 1
fi

echo "==> Fetching upstream..."
git fetch upstream --prune --tags

echo "==> Checking out main..."
git checkout main

echo "==> Hard-resetting main to upstream/main..."
git reset --hard upstream/main

echo "==> Updating submodules..."
git submodule update --init --recursive

head_sha=$(git rev-parse --short HEAD)
echo "==> Done. main is now at $head_sha."
