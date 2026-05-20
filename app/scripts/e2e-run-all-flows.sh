#!/usr/bin/env bash
#
# Run all E2E WDIO specs sequentially (Appium restarted per spec).
# Requires a prior E2E app build: pnpm --filter openhuman-app test:e2e:build
#
# Each spec runs to completion regardless of prior failures; a pass/fail
# summary is printed at the end and the script exits non-zero if any spec
# failed. (Previously `set -e` caused the first failure to abort the run
# and made the terminal appear to crash.)
#
set -uo pipefail

APP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$APP_DIR" || { echo "FATAL: could not cd to $APP_DIR" >&2; exit 1; }

# Parallel arrays: names + exit codes collected during the run.
_spec_names=()
_spec_results=()

run() {
  local spec="$1"
  local label="${2:-$1}"
  _spec_names+=("$label")
  if "$APP_DIR/scripts/e2e-run-spec.sh" "$spec" "$label"; then
    _spec_results+=(0)
  else
    _spec_results+=(1)
  fi
}

# Print summary and exit with the appropriate code.
finish() {
  local pass=0 fail=0
  echo ""
  echo "══════════════════════════════════════════════"
  echo "  E2E run summary  ($(uname -s))"
  echo "══════════════════════════════════════════════"
  for i in "${!_spec_names[@]}"; do
    if [[ "${_spec_results[$i]}" -eq 0 ]]; then
      printf "  ✓  %s\n" "${_spec_names[$i]}"
      (( pass++ )) || true
    else
      printf "  ✗  %s\n" "${_spec_names[$i]}"
      (( fail++ )) || true
    fi
  done
  echo "──────────────────────────────────────────────"
  printf "  Passed: %d   Failed: %d   Total: %d\n" "$pass" "$fail" "${#_spec_names[@]}"
  echo "══════════════════════════════════════════════"
  if [[ $fail -gt 0 ]]; then
    exit 1
  fi
}
trap finish EXIT

# ---------------------------------------------------------------------------
# Auth & onboarding
# ---------------------------------------------------------------------------
run "test/e2e/specs/smoke.spec.ts"                          "smoke"
run "test/e2e/specs/login-flow.spec.ts"                     "login"
run "test/e2e/specs/auth-access-control.spec.ts"            "auth"
run "test/e2e/specs/logout-relogin-onboarding.spec.ts"      "logout-relogin"
run "test/e2e/specs/onboarding-modes.spec.ts"               "onboarding-modes"
run "test/e2e/specs/runtime-picker-login.spec.ts"           "runtime-picker-login"

# ---------------------------------------------------------------------------
# Navigation & core UI
# ---------------------------------------------------------------------------
run "test/e2e/specs/navigation.spec.ts"                     "navigation"
run "test/e2e/specs/command-palette.spec.ts"                "command-palette"
run "test/e2e/specs/channels-smoke.spec.ts"                 "channels-smoke"
run "test/e2e/specs/insights-dashboard.spec.ts"             "insights-dashboard"

# ---------------------------------------------------------------------------
# Chat & agent harness
# ---------------------------------------------------------------------------
run "test/e2e/specs/chat-harness-send-stream.spec.ts"       "chat-send-stream"
run "test/e2e/specs/chat-harness-cancel.spec.ts"            "chat-cancel"
run "test/e2e/specs/chat-harness-scroll-render.spec.ts"     "chat-scroll-render"
run "test/e2e/specs/chat-harness-subagent.spec.ts"          "chat-subagent"
run "test/e2e/specs/chat-harness-wallet-flow.spec.ts"       "chat-wallet"
run "test/e2e/specs/agent-review.spec.ts"                   "agent-review"
run "test/e2e/specs/mega-flow.spec.ts"                      "mega-flow"

# ---------------------------------------------------------------------------
# Skills
# ---------------------------------------------------------------------------
run "test/e2e/specs/skills-registry.spec.ts"                "skills-registry"
run "test/e2e/specs/skill-execution-flow.spec.ts"           "skill-execution"
run "test/e2e/specs/skill-lifecycle.spec.ts"                "skill-lifecycle"
run "test/e2e/specs/skill-multi-round.spec.ts"              "skill-multi-round"
run "test/e2e/specs/skill-oauth.spec.ts"                    "skill-oauth"
run "test/e2e/specs/skill-socket-reconnect.spec.ts"         "skill-socket-reconnect"

# ---------------------------------------------------------------------------
# Notifications, memory, cron
# ---------------------------------------------------------------------------
run "test/e2e/specs/notifications.spec.ts"                  "notifications"
run "test/e2e/specs/memory-roundtrip.spec.ts"               "memory-roundtrip"
run "test/e2e/specs/cron-jobs-flow.spec.ts"                 "cron-jobs"
run "test/e2e/specs/autocomplete-flow.spec.ts"              "autocomplete"

# ---------------------------------------------------------------------------
# Webhooks & tools
# ---------------------------------------------------------------------------
run "test/e2e/specs/webhooks-ingress-flow.spec.ts"          "webhooks-ingress"
run "test/e2e/specs/webhooks-tunnel-flow.spec.ts"           "webhooks-tunnel"
run "test/e2e/specs/tool-browser-flow.spec.ts"              "tool-browser"
run "test/e2e/specs/tool-filesystem-flow.spec.ts"           "tool-filesystem"
run "test/e2e/specs/tool-shell-git-flow.spec.ts"            "tool-shell-git"

# ---------------------------------------------------------------------------
# Provider flows
# ---------------------------------------------------------------------------
run "test/e2e/specs/telegram-flow.spec.ts"                  "telegram"
run "test/e2e/specs/gmail-flow.spec.ts"                     "gmail"
run "test/e2e/specs/slack-flow.spec.ts"                     "slack"
run "test/e2e/specs/whatsapp-flow.spec.ts"                  "whatsapp"
run "test/e2e/specs/conversations-web-channel-flow.spec.ts" "conversations"
run "test/e2e/specs/composio-triggers-flow.spec.ts"         "composio-triggers"

# ---------------------------------------------------------------------------
# Payments & rewards
# ---------------------------------------------------------------------------
run "test/e2e/specs/card-payment-flow.spec.ts"              "card-payment"
run "test/e2e/specs/crypto-payment-flow.spec.ts"            "crypto-payment"
run "test/e2e/specs/rewards-unlock-flow.spec.ts"            "rewards-unlock"
run "test/e2e/specs/rewards-progression-persistence.spec.ts" "rewards-progression"

# ---------------------------------------------------------------------------
# Settings panels
# ---------------------------------------------------------------------------
run "test/e2e/specs/settings-channels-permissions.spec.ts"  "settings-channels"
run "test/e2e/specs/settings-data-management.spec.ts"       "settings-data"
run "test/e2e/specs/settings-dev-options.spec.ts"           "settings-dev"
run "test/e2e/specs/settings-ai-skills.spec.ts"             "settings-ai-skills"
run "test/e2e/specs/settings-account-preferences.spec.ts"   "settings-account"
run "test/e2e/specs/settings-advanced-config.spec.ts"       "settings-advanced"
run "test/e2e/specs/settings-feature-preferences.spec.ts"   "settings-features"

# ---------------------------------------------------------------------------
# AI, voice & screen
# ---------------------------------------------------------------------------
run "test/e2e/specs/local-model-runtime.spec.ts"            "local-model"
run "test/e2e/specs/voice-mode.spec.ts"                     "voice-mode"
run "test/e2e/specs/audio-toolkit-flow.spec.ts"             "audio-toolkit"

# ---------------------------------------------------------------------------
# System / Tauri
# ---------------------------------------------------------------------------
run "test/e2e/specs/tauri-commands.spec.ts"                 "tauri-commands"
OPENHUMAN_SERVICE_MOCK=1 \
  run "test/e2e/specs/service-connectivity-flow.spec.ts" "service-connectivity"

# linux-cef-deb-runtime.spec.ts is Linux-only (tests /usr/bin path resolution
# for .deb package installs) — skipped on macOS/Windows.
if [[ "$(uname -s)" == "Linux" ]]; then
  run "test/e2e/specs/linux-cef-deb-runtime.spec.ts" "linux-cef-deb-runtime"
fi
