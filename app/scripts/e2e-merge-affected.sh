#!/usr/bin/env bash
set -uo pipefail
APP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$APP_DIR" || exit 1

_names=()
_results=()
run() {
  local spec="$1" label="${2:-$1}"
  _names+=("$label")
  if timeout 600 "$APP_DIR/scripts/e2e-run-spec.sh" "$spec" "$label"; then
    _results+=(0)
  else
    _results+=(1)
  fi
}

run "test/e2e/specs/notifications.spec.ts"                  "notifications"
run "test/e2e/specs/settings-data-management.spec.ts"       "settings-data"
run "test/e2e/specs/slack-flow.spec.ts"                     "slack"
run "test/e2e/specs/tauri-commands.spec.ts"                 "tauri-commands"
run "test/e2e/specs/whatsapp-flow.spec.ts"                  "whatsapp"

echo ""
echo "========= MERGE-AFFECTED SPEC SUMMARY ========="
pass=0; fail=0
for i in "${!_names[@]}"; do
  if [[ "${_results[$i]}" -eq 0 ]]; then
    printf "  PASS  %s\n" "${_names[$i]}"; (( pass++ )) || true
  else
    printf "  FAIL  %s\n" "${_names[$i]}"; (( fail++ )) || true
  fi
done
echo "------------------------------------------------"
printf "  Passed: %d  Failed: %d  Total: %d\n" "$pass" "$fail" "${#_names[@]}"
