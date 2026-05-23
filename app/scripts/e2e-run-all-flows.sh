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

VALID_SUITES="auth navigation chat skills notifications webhooks providers payments settings system journeys all"
SUITE_VALID=0
for s in $VALID_SUITES; do
  [[ "$SUITE" == "$s" ]] && SUITE_VALID=1 && break
done
if [[ $SUITE_VALID -eq 0 ]]; then
  echo "Invalid suite: '$SUITE'. Valid values: $VALID_SUITES" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Artifacts directory
# ---------------------------------------------------------------------------
E2E_ARTIFACTS_DIR="${E2E_ARTIFACTS_DIR:-$APP_DIR/test/e2e/artifacts/$(date +%Y%m%d-%H%M%S)}"
export E2E_ARTIFACTS_DIR

# ---------------------------------------------------------------------------
# Run tracking: parallel arrays indexed by position.
# _spec_suite[i]    — suite name this spec belongs to
# _spec_names[i]    — human-readable label
# _spec_results[i]  — 0 (pass) or 1 (fail)
# _spec_duration[i] — wall-clock seconds (integer)
# ---------------------------------------------------------------------------
_spec_suite=()
_spec_names=()
_spec_results=()
_spec_duration=()

_BAILED=0
_RUN_START_EPOCH=$(date +%s)

# ---------------------------------------------------------------------------
# run SPEC LABEL SUITE
#
# Records start time, runs e2e-run-spec.sh, records end time and result.
# Respects --bail: once _BAILED=1 all subsequent run() calls are no-ops
# that record a synthetic skip (exit 2) so the finish summary is still full.
# ---------------------------------------------------------------------------
run() {
  local spec="$1"
  local label="${2:-$1}"
  local suite="${3:-unknown}"

  _spec_suite+=("$suite")
  _spec_names+=("$label")

  if [[ $_BAILED -eq 1 ]]; then
    _spec_results+=(2)  # 2 = skipped due to bail
    _spec_duration+=(0)
    return
  fi

  local t_start t_end duration
  t_start=$(date +%s)
  if "$APP_DIR/scripts/e2e-run-spec.sh" "$spec" "$label"; then
    _spec_results+=(0)
  else
    _spec_results+=(1)
    if [[ $BAIL -eq 1 ]]; then
      echo ""
      echo "[e2e-run-all-flows] --bail: stopping after first failure ($label)"
      _BAILED=1
    fi
    # Copy any failure logs into the artifacts directory
    _copy_failure_logs "$label"
  fi
  t_end=$(date +%s)
  duration=$(( t_end - t_start ))
  _spec_duration+=("$duration")
}

# ---------------------------------------------------------------------------
# _copy_failure_logs LABEL
# Copies /tmp/openhuman-e2e-app-*.log files into E2E_ARTIFACTS_DIR on failure.
# ---------------------------------------------------------------------------
_copy_failure_logs() {
  local label="$1"
  local logs
  logs=$(ls /tmp/openhuman-e2e-app-*.log 2>/dev/null || true)
  if [[ -z "$logs" ]]; then
    return
  fi
  mkdir -p "$E2E_ARTIFACTS_DIR"
  for f in $logs; do
    local dest="$E2E_ARTIFACTS_DIR/$(basename "$f" .log)-${label}.log"
    cp "$f" "$dest" 2>/dev/null || true
  done
  echo "[e2e-run-all-flows] Failure logs copied to $E2E_ARTIFACTS_DIR"
}

# ---------------------------------------------------------------------------
# _mini_summary SUITE_NAME
# Prints a one-line pass/fail summary for a completed suite.
# ---------------------------------------------------------------------------
_mini_summary() {
  local suite="$1"
  local pass=0 fail=0 skip=0
  for i in "${!_spec_names[@]}"; do
    if [[ "${_spec_suite[$i]}" != "$suite" ]]; then continue; fi
    case "${_spec_results[$i]:-2}" in
      0) (( pass++ )) || true ;;
      1) (( fail++ )) || true ;;
      2) (( skip++ )) || true ;;
    esac
  done
  local total=$(( pass + fail + skip ))
  if [[ $fail -gt 0 ]]; then
    printf "  [%s] %d/%d passed (%d failed)\n" "$suite" "$pass" "$total" "$fail"
  elif [[ $skip -gt 0 ]]; then
    printf "  [%s] %d/%d passed (%d skipped/bailed)\n" "$suite" "$pass" "$total" "$skip"
  else
    printf "  [%s] %d/%d passed\n" "$suite" "$pass" "$total"
  fi
}

# ---------------------------------------------------------------------------
# finish — print per-category table, totals, wall time, and hints.
# Writes a Markdown summary to /tmp/e2e-summary.txt for CI job summaries.
# ---------------------------------------------------------------------------
finish() {
  local t_end_epoch
  t_end_epoch=$(date +%s)
  local wall=$(( t_end_epoch - _RUN_START_EPOCH ))
  local wall_min=$(( wall / 60 ))
  local wall_sec=$(( wall % 60 ))

  local pass=0 fail=0 skip=0
  echo ""
  echo "══════════════════════════════════════════════════════════════════"
  printf "  E2E run summary  ($(uname -s))  suite=%s\n" "$SUITE"
  echo "══════════════════════════════════════════════════════════════════"

  # --- per-spec rows ---
  local prev_suite=""
  for i in "${!_spec_names[@]}"; do
    local cur_suite="${_spec_suite[$i]}"
    if [[ "$cur_suite" != "$prev_suite" ]]; then
      echo ""
      printf "  ## %s\n" "$cur_suite"
      prev_suite="$cur_suite"
    fi
    local dur="${_spec_duration[$i]:-0}"
    case "${_spec_results[$i]:-2}" in
      0)
        printf "    ✓  %-45s  %3ds\n" "${_spec_names[$i]}" "$dur"
        (( pass++ )) || true
        ;;
      1)
        printf "    ✗  %-45s  %3ds\n" "${_spec_names[$i]}" "$dur"
        (( fail++ )) || true
        ;;
      2)
        printf "    -  %-45s  (skipped/bailed)\n" "${_spec_names[$i]}"
        (( skip++ )) || true
        ;;
    esac
  done

  local total=$(( pass + fail + skip ))
  echo ""
  echo "──────────────────────────────────────────────────────────────────"
  printf "  Passed: %-4d  Failed: %-4d  Skipped: %-4d  Total: %d\n" \
    "$pass" "$fail" "$skip" "$total"
  printf "  Wall time: %dm %02ds\n" "$wall_min" "$wall_sec"
  echo "══════════════════════════════════════════════════════════════════"

  if [[ $fail -gt 0 ]]; then
    echo ""
    echo "  To re-run a single failing spec:"
    echo "    bash app/scripts/e2e-run-session.sh test/e2e/specs/SPEC.spec.ts"
    echo ""
    echo "  Artifacts (if any):"
    echo "    $E2E_ARTIFACTS_DIR"
    echo ""
  fi

  # --- write /tmp/e2e-summary.txt for CI job summary ---
  {
    printf "## E2E Results ($(uname -s)) — suite=%s\n\n" "$SUITE"
    printf "| Result | Count |\n"
    printf "|--------|-------|\n"
    printf "| Passed | %d |\n" "$pass"
    printf "| Failed | %d |\n" "$fail"
    printf "| Skipped | %d |\n" "$skip"
    printf "| **Total** | **%d** |\n" "$total"
    printf "\n**Wall time:** %dm %02ds\n\n" "$wall_min" "$wall_sec"

    if [[ $fail -gt 0 ]]; then
      printf "### Failed specs\n\n"
      for i in "${!_spec_names[@]}"; do
        if [[ "${_spec_results[$i]}" -eq 1 ]]; then
          printf -- "- \`%s\`\n" "${_spec_names[$i]}"
        fi
      done
      printf "\n"
    fi
  } > /tmp/e2e-summary.txt

  if [[ $fail -gt 0 ]]; then
    exit 1
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
  [[ "$SUITE" == "all" || "$SUITE" == "$1" ]]
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
  _mini_summary "webhooks"
fi

# ---------------------------------------------------------------------------
# Provider flows
# ---------------------------------------------------------------------------
if should_run_suite "providers"; then
  echo ""
  echo "## Running suite: providers"
  run "test/e2e/specs/telegram-flow.spec.ts"                  "telegram"                  "providers"
  run "test/e2e/specs/gmail-flow.spec.ts"                     "gmail"                     "providers"
  run "test/e2e/specs/accounts-provider-modal.spec.ts"        "accounts-providers"        "providers"
  run "test/e2e/specs/slack-flow.spec.ts"                     "slack"                     "providers"
  run "test/e2e/specs/whatsapp-flow.spec.ts"                  "whatsapp"                  "providers"
  # notion-flow.spec.ts was removed; skip to avoid "spec not found" failure.
  # run "test/e2e/specs/notion-flow.spec.ts"                  "notion"                    "providers"
  run "test/e2e/specs/conversations-web-channel-flow.spec.ts" "conversations"             "providers"
  run "test/e2e/specs/composio-triggers-flow.spec.ts"         "composio-triggers"         "providers"
  _mini_summary "providers"
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
