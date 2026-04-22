#!/usr/bin/env bash
# Bootstrap a fresh git worktree for OpenHuman dev.
#
# `git worktree add` only checks out the tree. Submodules, untracked env
# files, and the staged core binary under app/src-tauri/binaries/ don't come
# along — the app won't build until they do. Run this once per worktree.
#
# Usage: from inside the worktree, `bash scripts/worktree-bootstrap.sh`.

set -euo pipefail

WORKTREE_ROOT="$(git rev-parse --show-toplevel)"
MAIN_ROOT="$(git worktree list --porcelain | awk '/^worktree / { print $2; exit }')"

if [[ "$WORKTREE_ROOT" == "$MAIN_ROOT" ]]; then
  echo "[bootstrap] This IS the primary worktree — nothing to do." >&2
  exit 0
fi

echo "[bootstrap] worktree: $WORKTREE_ROOT"
echo "[bootstrap] main:     $MAIN_ROOT"

echo "[bootstrap] initializing submodules (tauri-cef, skills)..."
git -C "$WORKTREE_ROOT" submodule update --init --recursive

for rel in ".env" "app/.env.local"; do
  src="$MAIN_ROOT/$rel"
  dst="$WORKTREE_ROOT/$rel"
  if [[ -f "$src" && ! -e "$dst" ]]; then
    echo "[bootstrap] symlinking $rel from main"
    mkdir -p "$(dirname "$dst")"
    ln -s "$src" "$dst"
  fi
done

# Stage the core sidecar binary. Either symlink to main's staged copy (fast,
# but will run main's code) OR build fresh from this worktree (slow, runs
# this branch's code). Default to fresh build — the whole point of a
# worktree is testing divergent code.
BIN="$WORKTREE_ROOT/app/src-tauri/binaries/openhuman-core-aarch64-apple-darwin"
if [[ ! -e "$BIN" ]]; then
  echo "[bootstrap] building + staging core sidecar from this worktree..."
  mkdir -p "$(dirname "$BIN")"
  (cd "$WORKTREE_ROOT" && cargo build --bin openhuman-core)
  (cd "$WORKTREE_ROOT/app" && yarn core:stage)
fi

echo "[bootstrap] installing node_modules (needed for husky hooks + prettier)..."
(cd "$WORKTREE_ROOT" && yarn install)

echo "[bootstrap] ensuring vendored tauri-cli installed..."
(cd "$WORKTREE_ROOT/app" && yarn tauri:ensure)

echo "[bootstrap] done. launch with:  cd app && yarn dev:app"
