#!/usr/bin/env bash
# Bootstrap a fresh git worktree for OpenHuman dev.
#
# `git worktree add` only checks out the tree. Submodules, untracked env
# files don't come along — the app won't build until they do. Run this once
# per worktree.
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

echo "[bootstrap] installing node_modules (needed for husky hooks + prettier)..."
(cd "$WORKTREE_ROOT" && pnpm install)

echo "[bootstrap] ensuring vendored tauri-cli installed..."
(cd "$WORKTREE_ROOT/app" && pnpm tauri:ensure)

echo "[bootstrap] done. launch with:  cd app && pnpm dev:app"
