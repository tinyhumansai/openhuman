#!/usr/bin/env bash
# Run the ignored Rust tests whose source contract requires TinyMemory.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

FEATURES="${MODULE_TEST_FEATURES:-modules}"
LIST_OUTPUT="$(mktemp)"
trap 'rm -f "$LIST_OUTPUT"' EXIT

mapfile -t TEST_NAMES < <(node scripts/ci/list-module-gated-tests.mjs)
mapfile -t SKIPPED_NAMES < <(node scripts/ci/list-module-gated-tests.mjs --skipped)
if [ "${#TEST_NAMES[@]}" -eq 0 ]; then
  echo "::error::module-gated test inventory is empty; refusing a vacuous pass" >&2
  exit 1
fi

bash scripts/ci-cancel-aware.sh cargo test -p openhuman --lib --features "$FEATURES" -- --ignored --list >"$LIST_OUTPUT"

run_count=0
for name in "${TEST_NAMES[@]}"; do
  matches="$(sed -n 's/: test$//p' "$LIST_OUTPUT" | grep -F "::${name}" || true)"
  match_count="$(printf '%s\n' "$matches" | sed '/^$/d' | wc -l | tr -d ' ')"
  if [ "$match_count" -eq 0 ]; then
    echo "::error::module-gated test '$name' resolved to $match_count tests" >&2
    exit 1
  fi
  while IFS= read -r test_path; do
    [ -n "$test_path" ] || continue
    echo "running module-gated test: $test_path"
    bash scripts/ci-cancel-aware.sh cargo test -p openhuman --lib --features "$FEATURES" "$test_path" -- \
      --ignored --exact --test-threads=1
    run_count=$((run_count + 1))
  done <<< "$matches"
done

echo "module-gated Rust tests scheduled: $run_count"
echo "module-gated Rust tests skipped: ${#SKIPPED_NAMES[@]} (tinydocs artifact is not staged in this lane)"
