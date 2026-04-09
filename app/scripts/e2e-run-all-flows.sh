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

run "test/e2e/specs/macos-distribution.spec.ts" "macos-distribution"
run "test/e2e/specs/auth-session-management.spec.ts" "auth"
run "test/e2e/specs/permissions-system-access.spec.ts" "permissions-system-access"
run "test/e2e/specs/local-model-runtime.spec.ts" "local-model"
run "test/e2e/specs/system-resource-access.spec.ts" "system-resource-access"
run "test/e2e/specs/memory-system.spec.ts" "memory-system"
run "test/e2e/specs/automation-scheduling.spec.ts" "automation-scheduling"
run "test/e2e/specs/chat-interface-flow.spec.ts" "chat-interface"
run "test/e2e/specs/chat-skills-integrations.spec.ts" "chat-skills-integrations"
run "test/e2e/specs/login-flow.spec.ts" "login"
run "test/e2e/specs/telegram-flow.spec.ts" "telegram"
run "test/e2e/specs/discord-flow.spec.ts" "discord"
run "test/e2e/specs/gmail-flow.spec.ts" "gmail"
run "test/e2e/specs/notion-flow.spec.ts" "notion"
run "test/e2e/specs/screen-intelligence.spec.ts" "screen-intelligence"
run "test/e2e/specs/voice-mode.spec.ts" "voice-mode"
run "test/e2e/specs/text-autocomplete-flow.spec.ts" "text-autocomplete"

# run "test/e2e/specs/skills-registry.spec.ts" "skills-registry"
# run "test/e2e/specs/rewards-settings.spec.ts" "rewards-settings"
# run "test/e2e/specs/navigation.spec.ts" "navigation"

echo "All E2E flows completed."
