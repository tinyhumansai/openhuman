#!/usr/bin/env bash
# PR CI frontend unit-test lane — changed-files-only Vitest.
#
# Fast-lane policy (PRs targeting main): run only the tests related to the
# files the PR actually changed, via `vitest related` (static import graph —
# reliable here because dynamic imports are banned in app/src). Coverage is
# still written to app/coverage/lcov.info; the PR CI Gate's diff-cover step
# then enforces >= 80% on changed lines. Untested changed files still appear
# in the lcov report at 0% (vitest coverage.include is explicit), so the gate
# cannot be dodged by having no related tests.
#
# Inputs (env):
#   FULL          "true" → run the entire suite with coverage (config-level
#                 change: lockfile, vitest/vite/ts config, test setup, etc.)
#   CHANGED_FILES shell-quoted, space-separated repo-relative paths from
#                 dorny/paths-filter (list-files: shell)
#
# Falls back to the FULL suite whenever scoping is not clearly safe.
set -euo pipefail

FULL="${FULL:-false}"
CHANGED_FILES="${CHANGED_FILES:-}"
# Above this many changed source files a scoped run buys little and the
# argv/related-graph bookkeeping gets silly — just run everything.
MAX_RELATED_FILES="${MAX_RELATED_FILES:-200}"

log() { echo "[ci][vitest-changed] $*"; }

run_full() {
  log "running FULL Vitest coverage suite (reason: $1)"
  exec bash scripts/ci-cancel-aware.sh pnpm --filter openhuman-app test:coverage
}

if [ "${FULL}" = "true" ]; then
  run_full "config/workflow-level change detected by paths-filter"
fi

# CHANGED_FILES is the shell-quoted list from dorny/paths-filter
# (list-files: shell). Filenames are PR-controlled, so never eval it —
# xargs unquotes tokens as data without ever invoking a shell. If xargs
# can't parse it (e.g. hostile quoting), we get an empty list and fall
# back to the full suite.
declare -a files=()
while IFS= read -r f; do
  [ -n "${f}" ] && files+=("${f}")
done < <(printf '%s\n' "${CHANGED_FILES}" | xargs -n1 printf '%s\n' 2>/dev/null || true)
log "received ${#files[@]} changed frontend file(s)"

if [ "${#files[@]}" -eq 0 ]; then
  run_full "empty changed-file list — scoping unsafe"
fi

declare -a related=()
for f in "${files[@]}"; do
  case "${f}" in
    app/src/*.ts | app/src/*.tsx) ;;
    *)
      log "ignoring non-source path: ${f}"
      continue
      ;;
  esac
  if [ ! -f "${f}" ]; then
    log "skipping deleted/renamed file: ${f}"
    continue
  fi
  # vitest runs from app/ (config root) — strip the workspace prefix.
  related+=("${f#app/}")
done

if [ "${#related[@]}" -eq 0 ]; then
  run_full "no surviving changed .ts/.tsx files under app/src — scoping unsafe"
fi
if [ "${#related[@]}" -gt "${MAX_RELATED_FILES}" ]; then
  run_full "${#related[@]} changed files exceed MAX_RELATED_FILES=${MAX_RELATED_FILES}"
fi

log "running 'vitest related' with coverage for ${#related[@]} file(s):"
printf '[ci][vitest-changed]   %s\n' "${related[@]}"

cd app
exec bash ../scripts/ci-cancel-aware.sh pnpm exec vitest related --run --coverage --config test/vitest.config.ts "${related[@]}"
