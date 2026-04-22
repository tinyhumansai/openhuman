#!/usr/bin/env bash
#
# Run all E2E WDIO specs sequentially (Appium restarted per spec).
# Requires a prior E2E app build: yarn test:e2e:build
#
set -euo pipefail

APP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$APP_DIR"

run() {
  "$APP_DIR/scripts/e2e-run-spec.sh" "$1" "$2"
}

run "test/e2e/specs/login-flow.spec.ts" "login"
run "test/e2e/specs/auth-access-control.spec.ts" "auth"
run "test/e2e/specs/telegram-flow.spec.ts" "telegram"
run "test/e2e/specs/gmail-flow.spec.ts" "gmail"
run "test/e2e/specs/notion-flow.spec.ts" "notion"
run "test/e2e/specs/card-payment-flow.spec.ts" "card-payment"
run "test/e2e/specs/crypto-payment-flow.spec.ts" "crypto-payment"
run "test/e2e/specs/conversations-web-channel-flow.spec.ts" "conversations"
run "test/e2e/specs/local-model-runtime.spec.ts" "local-model"
run "test/e2e/specs/screen-intelligence.spec.ts" "screen-intelligence"
OPENHUMAN_SERVICE_MOCK=1 run "test/e2e/specs/service-connectivity-flow.spec.ts" "service-connectivity"
run "test/e2e/specs/skills-registry.spec.ts" "skills-registry"
run "test/e2e/specs/skill-execution-flow.spec.ts" "skill-execution"
run "test/e2e/specs/cron-jobs-flow.spec.ts" "cron-jobs"
run "test/e2e/specs/navigation.spec.ts" "navigation"
run "test/e2e/specs/smoke.spec.ts" "smoke"
run "test/e2e/specs/tauri-commands.spec.ts" "tauri-commands"

echo "All E2E flows completed."
