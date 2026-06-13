#!/usr/bin/env bash
#
# e2e-run-all-flows.sh — Master E2E orchestrator for all 66 WDIO specs.
#
# USAGE:
#   bash app/scripts/e2e-run-all-flows.sh [OPTIONS]
#
# OPTIONS:
#   --suite=SUITE     Run only one suite category. Valid values:
#                       auth, navigation, chat, skills, notifications,
#                       webhooks, providers, payments, settings, system,
#                       journeys, all  (default: all)
#   --bail            Stop after the first spec failure (default: run all)
#   --skip-preflight  Skip the pre-flight environment check
#
# ENVIRONMENT:
#   E2E_ARTIFACTS_DIR  Directory where failure logs are copied.
#                      Default: app/test/e2e/artifacts/YYYYMMDD-HHMMSS
#
# REQUIREMENTS:
#   pnpm --filter openhuman-app test:e2e:build   (must be run first)
#
# Each spec runs to completion regardless of prior failures unless --bail is
# passed. A per-category mini-summary and a full summary are printed at the
# end. The script exits non-zero if any spec failed.
#
# (Previously `set -e` caused the first failure to abort the run and made
# the terminal appear to crash. `set -uo pipefail` preserves error detection
# without aborting mid-run.)
#
set -uo pipefail

APP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_DIR="$(cd "$APP_DIR/.." && pwd)"
cd "$APP_DIR" || {
  echo "[e2e-run-all-flows] Failed to cd into $APP_DIR" >&2
  exit 1
}

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
SUITE="all"
BAIL=0
SKIP_PREFLIGHT=0

for arg in "$@"; do
  case "$arg" in
    --suite=*)  SUITE="${arg#--suite=}" ;;
    --bail)     BAIL=1 ;;
    --skip-preflight) SKIP_PREFLIGHT=1 ;;
    *)
      echo "Unknown option: $arg" >&2
      echo "Usage: bash app/scripts/e2e-run-all-flows.sh [--suite=SUITE] [--bail] [--skip-preflight]" >&2
      exit 1
      ;;
  esac
done

VALID_SUITES="auth navigation chat skills notifications webhooks providers connectors payments settings system journeys all"

# Accept comma-separated suite lists, e.g. --suite=auth,navigation,system.
# CI sharding passes one such list per matrix shard so a few parallel jobs
# can cover the whole suite. `all` short-circuits to "everything".
IFS=',' read -r -a _REQUESTED_SUITES <<< "$SUITE"
for req in "${_REQUESTED_SUITES[@]}"; do
  match=0
  for s in $VALID_SUITES; do
    [[ "$req" == "$s" ]] && match=1 && break
  done
  if [[ $match -eq 0 ]]; then
    echo "Invalid suite: '$req'. Valid values: $VALID_SUITES" >&2
    exit 1
  fi
done

# ---------------------------------------------------------------------------
# Artifacts directory
# ---------------------------------------------------------------------------
E2E_ARTIFACTS_DIR="${E2E_ARTIFACTS_DIR:-$APP_DIR/test/e2e/artifacts/$(date +%Y%m%d-%H%M%S)}"
export E2E_ARTIFACTS_DIR

# ---------------------------------------------------------------------------
# Spec collection: this script no longer invokes the runner once per spec.
# Instead `run()` accumulates spec paths into one list; at the very end we
# hand the whole list to `e2e-run-session.sh`, which launches the app +
# Appium + chromedriver ONCE and lets WDIO drive every spec inside a single
# shared session. The old per-spec orchestration paid CEF cold-start tax on
# every spec (~15-30s × 65 specs) and broke the contract in wdio.conf.ts
# ("WDIO creates ONE session per worker ... state from spec N flows into
# spec N+1"). Per-spec failure detail comes from WDIO's spec reporter now,
# not from a bash-side per-spec exit-code table.
#
# `--bail` is forwarded by env (E2E_BAIL_ON_FAILURE=1) so wdio.conf.ts can
# flip its `bail` count. Per-suite `--suite=` filtering is still honored at
# the run() call site.
# ---------------------------------------------------------------------------
_spec_paths=()    # collected spec paths, in declaration order
_spec_suites=()   # parallel array: suite name per collected spec
_spec_labels=()   # parallel array: human label per collected spec
_RUN_START_EPOCH=$(date +%s)

# ---------------------------------------------------------------------------
# run SPEC LABEL SUITE
#
# Appends the spec to the collected list; nothing runs yet. The actual WDIO
# invocation happens at the bottom of the script.
# ---------------------------------------------------------------------------
run() {
  local spec="$1"
  local label="${2:-$1}"
  local suite="${3:-unknown}"

  _spec_paths+=("$spec")
  _spec_suites+=("$suite")
  _spec_labels+=("$label")
}

# ---------------------------------------------------------------------------
# _copy_failure_logs
# Copies /tmp/openhuman-e2e-app-*.log files into E2E_ARTIFACTS_DIR once at
# end-of-run. With a single shared session there's now only one app log to
# capture (and Appium/chromedriver logs alongside).
# ---------------------------------------------------------------------------
_copy_failure_logs() {
  local logs
  logs=$(ls /tmp/openhuman-e2e-app-*.log 2>/dev/null || true)
  if [[ -z "$logs" ]]; then
    return
  fi
  mkdir -p "$E2E_ARTIFACTS_DIR"
  for f in $logs; do
    local dest="$E2E_ARTIFACTS_DIR/$(basename "$f" .log)-session.log"
    cp "$f" "$dest" 2>/dev/null || true
  done
  echo "[e2e-run-all-flows] Session logs copied to $E2E_ARTIFACTS_DIR"
}

# ---------------------------------------------------------------------------
# _mini_summary SUITE_NAME
# Print how many specs were collected for this suite (pre-run; WDIO will
# report per-spec pass/fail directly).
# ---------------------------------------------------------------------------
_mini_summary() {
  local suite="$1"
  local count=0
  for i in "${!_spec_labels[@]}"; do
    [[ "${_spec_suites[$i]}" == "$suite" ]] && (( count++ )) || true
  done
  printf "  [%s] %d spec(s) queued\n" "$suite" "$count"
}

# ---------------------------------------------------------------------------
# finish — print wall time + a markdown summary for CI job summary.
# Per-spec pass/fail comes from WDIO's spec reporter in the live output;
# the bash orchestrator no longer tracks per-spec exit codes.
# ---------------------------------------------------------------------------
_WDIO_EXIT_CODE=0
finish() {
  local t_end_epoch
  t_end_epoch=$(date +%s)
  local wall=$(( t_end_epoch - _RUN_START_EPOCH ))
  local wall_min=$(( wall / 60 ))
  local wall_sec=$(( wall % 60 ))
  local collected=${#_spec_paths[@]}

  echo ""
  echo "══════════════════════════════════════════════════════════════════"
  printf "  E2E run summary  ($(uname -s))  suite=%s\n" "$SUITE"
  echo "══════════════════════════════════════════════════════════════════"
  printf "  Specs queued: %d\n" "$collected"
  printf "  WDIO exit:    %d\n" "$_WDIO_EXIT_CODE"
  printf "  Wall time:    %dm %02ds\n" "$wall_min" "$wall_sec"
  echo "══════════════════════════════════════════════════════════════════"

  _copy_failure_logs

  {
    printf "## E2E Results ($(uname -s)) — suite=%s\n\n" "$SUITE"
    printf "| Field | Value |\n"
    printf "|-------|-------|\n"
    printf "| Specs queued | %d |\n" "$collected"
    printf "| WDIO exit code | %d |\n" "$_WDIO_EXIT_CODE"
    printf "| Wall time | %dm %02ds |\n" "$wall_min" "$wall_sec"
    printf "\nPer-spec pass/fail is in the WDIO spec-reporter output above.\n"
  } > /tmp/e2e-summary.txt

  if [[ $_WDIO_EXIT_CODE -ne 0 ]]; then
    exit "$_WDIO_EXIT_CODE"
  fi
}
trap finish EXIT

# ---------------------------------------------------------------------------
# Pre-flight check (unless --skip-preflight)
# ---------------------------------------------------------------------------
if [[ $SKIP_PREFLIGHT -eq 0 ]]; then
  if [[ -f "$APP_DIR/scripts/e2e-preflight.sh" ]]; then
    echo "[e2e-run-all-flows] Running pre-flight checks..."
    if ! bash "$APP_DIR/scripts/e2e-preflight.sh"; then
      echo "[e2e-run-all-flows] Pre-flight failed. Aborting." >&2
      exit 1
    fi
  else
    echo "[e2e-run-all-flows] Pre-flight script not found or not executable, skipping."
  fi
fi

# ---------------------------------------------------------------------------
# Helpers: should_run_suite SUITE_NAME
# Returns 0 (true) if this suite should run given --suite flag.
# ---------------------------------------------------------------------------
should_run_suite() {
  local want="$1"
  for req in "${_REQUESTED_SUITES[@]}"; do
    [[ "$req" == "all" || "$req" == "$want" ]] && return 0
  done
  return 1
}

# ---------------------------------------------------------------------------
# Auth & onboarding
# ---------------------------------------------------------------------------
if should_run_suite "auth"; then
  echo ""
  echo "## Running suite: auth"
  run "test/e2e/specs/smoke.spec.ts"                          "smoke"                     "auth"
  run "test/e2e/specs/login-flow.spec.ts"                     "login"                     "auth"
  run "test/e2e/specs/auth-access-control.spec.ts"            "auth"                      "auth"
  run "test/e2e/specs/logout-relogin-onboarding.spec.ts"      "logout-relogin"            "auth"
  run "test/e2e/specs/onboarding-modes.spec.ts"               "onboarding-modes"          "auth"
  run "test/e2e/specs/runtime-picker-login.spec.ts"           "runtime-picker-login"      "auth"
  _mini_summary "auth"
fi

# ---------------------------------------------------------------------------
# Navigation & core UI
# ---------------------------------------------------------------------------
if should_run_suite "navigation"; then
  echo ""
  echo "## Running suite: navigation"
  run "test/e2e/specs/navigation.spec.ts"                     "navigation"                "navigation"
  run "test/e2e/specs/navigation-smoothness.spec.ts"          "navigation-smoothness"     "navigation"
  run "test/e2e/specs/navigation-settings-panels.spec.ts"     "navigation-settings"       "navigation"
  run "test/e2e/specs/command-palette.spec.ts"                "command-palette"           "navigation"
  run "test/e2e/specs/channels-smoke.spec.ts"                 "channels-smoke"            "navigation"
  run "test/e2e/specs/insights-dashboard.spec.ts"             "insights-dashboard"        "navigation"
  run "test/e2e/specs/guided-tour-gates.spec.ts"              "guided-tour-gates"         "navigation"
  _mini_summary "navigation"
fi

# ---------------------------------------------------------------------------
# Chat & agent harness
# ---------------------------------------------------------------------------
if should_run_suite "chat"; then
  echo ""
  echo "## Running suite: chat"
  run "test/e2e/specs/chat-harness-send-stream.spec.ts"       "chat-send-stream"          "chat"
  run "test/e2e/specs/chat-harness-cancel.spec.ts"            "chat-cancel"               "chat"
  run "test/e2e/specs/chat-harness-scroll-render.spec.ts"     "chat-scroll-render"        "chat"
  run "test/e2e/specs/chat-harness-subagent.spec.ts"          "chat-subagent"             "chat"
  run "test/e2e/specs/chat-harness-wallet-flow.spec.ts"       "chat-wallet"               "chat"
  run "test/e2e/specs/chat-tool-call-flow.spec.ts"            "chat-tool-call"            "chat"
  run "test/e2e/specs/chat-multi-tool-round.spec.ts"          "chat-multi-tool"           "chat"
  run "test/e2e/specs/chat-tool-error-recovery.spec.ts"       "chat-error-recovery"       "chat"
  run "test/e2e/specs/agent-review.spec.ts"                   "agent-review"              "chat"
  run "test/e2e/specs/mega-flow.spec.ts"                      "mega-flow"                 "chat"
  _mini_summary "chat"
fi

# ---------------------------------------------------------------------------
# Skills
# ---------------------------------------------------------------------------
if should_run_suite "skills"; then
  echo ""
  echo "## Running suite: skills"
  run "test/e2e/specs/skills-registry.spec.ts"                "skills-registry"           "skills"
  run "test/e2e/specs/skill-execution-flow.spec.ts"           "skill-execution"           "skills"
  run "test/e2e/specs/skill-lifecycle.spec.ts"                "skill-lifecycle"           "skills"
  run "test/e2e/specs/skill-multi-round.spec.ts"              "skill-multi-round"         "skills"
  run "test/e2e/specs/skill-oauth.spec.ts"                    "skill-oauth"               "skills"
  run "test/e2e/specs/skill-socket-reconnect.spec.ts"         "skill-socket-reconnect"    "skills"
  _mini_summary "skills"
fi

# ---------------------------------------------------------------------------
# Notifications, memory, cron
# ---------------------------------------------------------------------------
if should_run_suite "notifications"; then
  echo ""
  echo "## Running suite: notifications"
  run "test/e2e/specs/notifications.spec.ts"                  "notifications"             "notifications"
  run "test/e2e/specs/memory-roundtrip.spec.ts"               "memory-roundtrip"          "notifications"
  run "test/e2e/specs/cron-jobs-flow.spec.ts"                 "cron-jobs"                 "notifications"
  run "test/e2e/specs/autocomplete-flow.spec.ts"              "autocomplete"              "notifications"
  _mini_summary "notifications"
fi

# ---------------------------------------------------------------------------
# Webhooks & tools
# ---------------------------------------------------------------------------
if should_run_suite "webhooks"; then
  echo ""
  echo "## Running suite: webhooks"
  run "test/e2e/specs/webhooks-ingress-flow.spec.ts"          "webhooks-ingress"          "webhooks"
  run "test/e2e/specs/webhooks-tunnel-flow.spec.ts"           "webhooks-tunnel"           "webhooks"
  run "test/e2e/specs/tool-browser-flow.spec.ts"              "tool-browser"              "webhooks"
  run "test/e2e/specs/tool-filesystem-flow.spec.ts"           "tool-filesystem"           "webhooks"
  run "test/e2e/specs/tool-shell-git-flow.spec.ts"            "tool-shell-git"            "webhooks"
  run "test/e2e/specs/harness-channel-bridge-flow.spec.ts"    "harness-channel-bridge"    "webhooks"
  run "test/e2e/specs/harness-composio-tool-flow.spec.ts"     "harness-composio-tool"     "webhooks"
  run "test/e2e/specs/harness-cron-prompt-flow.spec.ts"       "harness-cron-prompt"       "webhooks"
  run "test/e2e/specs/harness-search-tool-flow.spec.ts"       "harness-search-tool"       "webhooks"
  _mini_summary "webhooks"
fi

# ---------------------------------------------------------------------------
# Provider flows
# ---------------------------------------------------------------------------
if should_run_suite "providers"; then
  echo ""
  echo "## Running suite: providers"
  # telegram-flow.spec.ts was renamed to telegram-channel-flow.spec.ts;
  # only the latter exists in the repo today.
  run "test/e2e/specs/telegram-channel-flow.spec.ts"          "telegram-channel"          "providers"
  run "test/e2e/specs/gmail-flow.spec.ts"                     "gmail"                     "providers"
  run "test/e2e/specs/accounts-provider-modal.spec.ts"        "accounts-providers"        "providers"
  # slack-flow currently crashes the CEF session mid-spec on Linux (#1850-style
  # state issue); skip until investigated rather than nuke the rest of the
  # provider suite.
  # run "test/e2e/specs/slack-flow.spec.ts"                   "slack"                     "providers"
  run "test/e2e/specs/whatsapp-flow.spec.ts"                  "whatsapp"                  "providers"
  # notion-flow.spec.ts was removed; skip to avoid "spec not found" failure.
  # run "test/e2e/specs/notion-flow.spec.ts"                  "notion"                    "providers"
  run "test/e2e/specs/conversations-web-channel-flow.spec.ts" "conversations"             "providers"
  run "test/e2e/specs/composio-triggers-flow.spec.ts"         "composio-triggers"         "providers"
  run "test/e2e/specs/connectivity-state-differentiation.spec.ts" "connectivity-state"   "providers"
  _mini_summary "providers"
fi

# ---------------------------------------------------------------------------
# Composio connector smoke specs.
#
# Split out of the `providers` suite into its own `connectors` shard so the
# 17 connector specs don't share a CEF session with the heavier provider
# flows (slack/whatsapp/etc.). The shared CEF process leaks resources over
# ~30+ specs and the second half of the suite hits 'A sessionId is
# required' / __simulateDeepLink-not-ready errors mid-run.
# ---------------------------------------------------------------------------
if should_run_suite "connectors"; then
  echo ""
  echo "## Running suite: connectors"
  run "test/e2e/specs/connector-airtable.spec.ts"            "connector-airtable"        "connectors"
  run "test/e2e/specs/connector-asana.spec.ts"               "connector-asana"           "connectors"
  run "test/e2e/specs/connector-clickup.spec.ts"             "connector-clickup"         "connectors"
  run "test/e2e/specs/connector-confluence.spec.ts"          "connector-confluence"      "connectors"
  run "test/e2e/specs/connector-discord-composio.spec.ts"    "connector-discord"         "connectors"
  run "test/e2e/specs/connector-github.spec.ts"              "connector-github"          "connectors"
  run "test/e2e/specs/connector-gmail-composio.spec.ts"      "connector-gmail-composio"  "connectors"
  run "test/e2e/specs/connector-google-calendar.spec.ts"     "connector-gcal"            "connectors"
  run "test/e2e/specs/connector-google-drive.spec.ts"        "connector-gdrive"          "connectors"
  run "test/e2e/specs/connector-google-sheets.spec.ts"       "connector-gsheets"         "connectors"
  run "test/e2e/specs/connector-jira.spec.ts"                "connector-jira"            "connectors"
  run "test/e2e/specs/connector-notion.spec.ts"              "connector-notion"          "connectors"
  run "test/e2e/specs/connector-session-guard.spec.ts"       "connector-session-guard"   "connectors"
  run "test/e2e/specs/connector-slack-composio.spec.ts"      "connector-slack-composio"  "connectors"
  run "test/e2e/specs/connector-todoist.spec.ts"             "connector-todoist"         "connectors"
  run "test/e2e/specs/connector-youtube.spec.ts"             "connector-youtube"         "connectors"
  _mini_summary "connectors"
fi

# ---------------------------------------------------------------------------
# Payments & rewards
# ---------------------------------------------------------------------------
if should_run_suite "payments"; then
  echo ""
  echo "## Running suite: payments"
  run "test/e2e/specs/card-payment-flow.spec.ts"              "card-payment"              "payments"
  run "test/e2e/specs/crypto-payment-flow.spec.ts"            "crypto-payment"            "payments"
  run "test/e2e/specs/rewards-unlock-flow.spec.ts"            "rewards-unlock"            "payments"
  run "test/e2e/specs/rewards-progression-persistence.spec.ts" "rewards-progression"      "payments"
  _mini_summary "payments"
fi

# ---------------------------------------------------------------------------
# Settings panels
# ---------------------------------------------------------------------------
if should_run_suite "settings"; then
  echo ""
  echo "## Running suite: settings"
  run "test/e2e/specs/settings-channels-permissions.spec.ts"  "settings-channels"         "settings"
  run "test/e2e/specs/settings-data-management.spec.ts"       "settings-data"             "settings"
  run "test/e2e/specs/settings-dev-options.spec.ts"           "settings-dev"              "settings"
  run "test/e2e/specs/settings-ai-skills.spec.ts"             "settings-ai-skills"        "settings"
  run "test/e2e/specs/settings-account-preferences.spec.ts"   "settings-account"          "settings"
  run "test/e2e/specs/settings-advanced-config.spec.ts"       "settings-advanced"         "settings"
  run "test/e2e/specs/settings-feature-preferences.spec.ts"   "settings-features"         "settings"
  _mini_summary "settings"
fi

# ---------------------------------------------------------------------------
# System / AI / voice / screen / Tauri
# linux-cef-deb-runtime.spec.ts is Linux-only (tests /usr/bin path resolution
# for .deb package installs) — skipped on macOS/Windows.
# ---------------------------------------------------------------------------
if should_run_suite "system"; then
  echo ""
  echo "## Running suite: system"
  run "test/e2e/specs/local-model-runtime.spec.ts"            "local-model"               "system"
  run "test/e2e/specs/voice-mode.spec.ts"                     "voice-mode"                "system"
  run "test/e2e/specs/screen-intelligence.spec.ts"            "screen-intelligence"       "system"
  run "test/e2e/specs/audio-toolkit-flow.spec.ts"             "audio-toolkit"             "system"
  run "test/e2e/specs/tauri-commands.spec.ts"                 "tauri-commands"            "system"
  # service-connectivity-flow tests the old sidecar service model removed in
  # PR #1061 (core is now in-process). Skip by not setting OPENHUMAN_SERVICE_MOCK=1.
  run "test/e2e/specs/service-connectivity-flow.spec.ts"    "service-connectivity"      "system"
  run "test/e2e/specs/core-port-conflict-recovery.spec.ts"  "core-port-conflict"        "system"
  if [[ "$(uname -s)" == "Linux" ]]; then
    run "test/e2e/specs/linux-cef-deb-runtime.spec.ts"        "linux-cef-deb-runtime"     "system"
  fi
  _mini_summary "system"
fi

# ---------------------------------------------------------------------------
# User journeys
# ---------------------------------------------------------------------------
if should_run_suite "journeys"; then
  echo ""
  echo "## Running suite: journeys"
  run "test/e2e/specs/user-journey-full-task.spec.ts"              "journey-full-task"     "journeys"
  run "test/e2e/specs/user-journey-settings-round-trip.spec.ts"    "journey-settings"      "journeys"
  run "test/e2e/specs/chat-conversation-history.spec.ts"           "chat-history"          "journeys"
  _mini_summary "journeys"
fi

# ---------------------------------------------------------------------------
# Single shared WDIO session.
#
# All collected specs run inside one Appium/CEF session, restoring the
# contract in wdio.conf.ts. Per-spec pass/fail comes from WDIO's spec
# reporter (live stdout above). Exit code from e2e-run-session.sh is
# propagated to the `finish` summary trap.
#
# `--bail` is forwarded via E2E_BAIL_ON_FAILURE (wdio.conf.ts flips its
# `bail` count when this env is set).
# ---------------------------------------------------------------------------
if [[ ${#_spec_paths[@]} -eq 0 ]]; then
  echo "[e2e-run-all-flows] no specs matched suite=$SUITE — nothing to run." >&2
  exit 1
fi

echo ""
echo "──────────────────────────────────────────────────────────────────"
echo "  Launching single shared WDIO session for ${#_spec_paths[@]} spec(s)"
echo "──────────────────────────────────────────────────────────────────"

if [[ $BAIL -eq 1 ]]; then
  export E2E_BAIL_ON_FAILURE=1
fi

set +e
bash "$APP_DIR/scripts/e2e-run-session.sh" "${_spec_paths[@]}"
_WDIO_EXIT_CODE=$?
set -e

# finish() trap will print the summary and exit with _WDIO_EXIT_CODE.
