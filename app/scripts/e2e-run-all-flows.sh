#!/usr/bin/env bash
#
# Run all E2E WDIO specs sequentially (Appium restarted per spec).
# Requires a prior E2E app build: yarn test:e2e:build
#
# Failure policy: specs are independent, so one failing spec must NOT abort
# subsequent specs. We collect every failure and exit non-zero at the end
# with a summary, so CI sees the full picture instead of bailing on spec #1.
#
set -uo pipefail

APP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$APP_DIR"

FAILED_SPECS=()
PASSED_SPECS=()

run() {
  local spec="$1"
  local label="$2"
  echo ""
  echo "============================================================"
  echo "[e2e-run-all-flows] START $label ($spec)"
  echo "============================================================"
  if "$APP_DIR/scripts/e2e-run-spec.sh" "$spec" "$label"; then
    echo "[e2e-run-all-flows] PASS  $label"
    PASSED_SPECS+=("$label")
  else
    local rc=$?
    echo "[e2e-run-all-flows] FAIL  $label (exit=$rc)"
    FAILED_SPECS+=("$label")
  fi
}

run "test/e2e/specs/macos-distribution.spec.ts" "macos-distribution"
run "test/e2e/specs/auth-session-management.spec.ts" "auth"
run "test/e2e/specs/permissions-system-access.spec.ts" "permissions-system-access"
# run "test/e2e/specs/local-model-runtime.spec.ts" "local-model"
run "test/e2e/specs/system-resource-access.spec.ts" "system-resource-access"
# run "test/e2e/specs/service-connectivity-flow.spec.ts" "service-connectivity"
run "test/e2e/specs/memory-system.spec.ts" "memory-system"
run "test/e2e/specs/automation-scheduling.spec.ts" "automation-scheduling"
run "test/e2e/specs/chat-interface-flow.spec.ts" "chat-interface"
run "test/e2e/specs/login-flow.spec.ts" "login"
run "test/e2e/specs/telegram-flow.spec.ts" "telegram"
run "test/e2e/specs/discord-flow.spec.ts" "discord"
run "test/e2e/specs/gmail-flow.spec.ts" "gmail"
run "test/e2e/specs/notion-flow.spec.ts" "notion"
run "test/e2e/specs/screen-intelligence.spec.ts" "screen-intelligence"
run "test/e2e/specs/voice-mode.spec.ts" "voice-mode"
run "test/e2e/specs/text-autocomplete-flow.spec.ts" "text-autocomplete"
run "test/e2e/specs/rewards-flow.spec.ts" "rewards-flow"
run "test/e2e/specs/settings-flow.spec.ts" "settings-flow"

echo ""
echo "============================================================"
echo "[e2e-run-all-flows] SUMMARY"
echo "============================================================"
echo "  Passed (${#PASSED_SPECS[@]}): ${PASSED_SPECS[*]:-<none>}"
echo "  Failed (${#FAILED_SPECS[@]}): ${FAILED_SPECS[*]:-<none>}"

if [ "${#FAILED_SPECS[@]}" -gt 0 ]; then
  echo "[e2e-run-all-flows] One or more specs failed — exiting non-zero."
  exit 1
fi

echo "All E2E flows completed."
