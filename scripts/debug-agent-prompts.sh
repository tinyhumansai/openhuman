#!/usr/bin/env bash
#
# debug-agent-prompts.sh — Dump the exact system prompt the context engine
# would produce for every built-in agent (plus the main / orchestrator
# agent), so prompt-engineering changes can be reviewed in one place.
#
# Each prompt is written to a numbered file under the output directory
# along with a side-car `.meta.txt` containing the metadata banner
# (agent id, model, tool count, cache boundary, …) that the CLI prints
# to stderr. Useful workflow:
#
#   bash scripts/debug-agent-prompts.sh
#   diff -u prompts.before/skills_agent.md prompts.after/skills_agent.md
#
# When run with `--stub-composio` (the default) the dumper injects the
# five Composio meta-tools into the registry even on machines that are
# not signed in, so `skills_agent` always renders the full Composio
# surface. Pass `--no-stub-composio` to see the raw on-disk state instead
# (useful for sanity-checking the unauthed onboarding path).
#
# The dumper runs against the currently-logged-in user's workspace
# (`$OPENHUMAN_WORKSPACE`, falling back to `~/.openhuman/workspace`) so
# onboarding-generated files like `PROFILE.md` appear in the dump. Export
# `OPENHUMAN_WORKSPACE=<path>` before running if you want to target a
# different workspace.
#
# Usage:
#   bash scripts/debug-agent-prompts.sh [--out <dir>] [--no-stub-composio] [--with-tools] [-v]
#
# The output directory is wiped and recreated at the start of each run
# so the snapshot only reflects the current agent set — stale files from
# an earlier run cannot hide a regression.
#
# Defaults:
#   --out                  ./prompt-dumps   (deleted + recreated each run)
#   --stub-composio        ON (override with --no-stub-composio)
#   --with-tools           OFF (pass to also list each agent's tool names)
#

set -euo pipefail

# ── Locate repo root + binary ─────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
BIN="${REPO_ROOT}/target/debug/openhuman-core"

# Always run `cargo build` — it no-ops when the binary is already
# up-to-date, and re-links quickly when it isn't. The old `-x` existence
# check let a stale debug binary survive across agent-registry changes
# (e.g. new entries in `agents::BUILTINS`), which made this script
# silently skip newly added agents like `welcome`.
echo "[debug-agent-prompts] building openhuman-core (no-op if up-to-date) …" >&2
( cd "${REPO_ROOT}" && cargo build --manifest-path Cargo.toml --bin openhuman-core >&2 )

# ── Parse flags ───────────────────────────────────────────────────────────
OUT_DIR=""
STUB_COMPOSIO=1
WITH_TOOLS=0
VERBOSE_FLAG=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out)
      if [[ -z "${2-}" ]] || [[ "${2-}" == -* ]]; then
        echo "[debug-agent-prompts] missing value for --out" >&2
        exit 64
      fi
      OUT_DIR="$2"
      shift 2
      ;;
    --stub-composio)
      STUB_COMPOSIO=1
      shift
      ;;
    --no-stub-composio)
      STUB_COMPOSIO=0
      shift
      ;;
    --with-tools)
      WITH_TOOLS=1
      shift
      ;;
    -v|--verbose)
      VERBOSE_FLAG=(-v)
      shift
      ;;
    -h|--help)
      sed -n '2,30p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "[debug-agent-prompts] unknown flag: $1" >&2
      exit 64
      ;;
  esac
done

if [[ -z "${OUT_DIR}" ]]; then
  OUT_DIR="${REPO_ROOT}/prompt-dumps"
fi

# ── Validate & canonicalize OUT_DIR before `rm -rf` ─────────────────────
# The output directory is wiped at the start of each run. Literal string
# matching against "/" / $HOME / $REPO_ROOT is not enough on its own:
# trailing slashes, ".", "..", or symlinked paths can slip past and
# trigger `rm -rf` on a sensitive target. So:
#
#   1. Reject obviously bad inputs up-front ("", ".", "..", relative).
#   2. Canonicalize OUT_DIR and REPO_ROOT via `realpath` (falling back
#      to python when realpath is unavailable on barebones macOS).
#   3. Match the canonicalized form against the disallow list.
#   4. Only then `rm -rf` the canonicalized path.
case "${OUT_DIR}" in
  "" | "." | "..")
    echo "[debug-agent-prompts] refusing to wipe --out='${OUT_DIR}' (relative/empty)" >&2
    exit 64
    ;;
esac
if [[ "${OUT_DIR}" != /* ]]; then
  echo "[debug-agent-prompts] --out must be an absolute path (starts with '/'), got '${OUT_DIR}'" >&2
  exit 64
fi

canonicalize() {
  local p="$1"
  # `realpath` is GNU + modern macOS (coreutils), and `readlink -f` on
  # Linux. Try both; if neither resolves the path (target missing) we
  # fall back to python3, which handles symlinks even for non-existent
  # leaves via `os.path.realpath`.
  if command -v realpath >/dev/null 2>&1; then
    realpath -m -- "${p}" 2>/dev/null && return 0
  fi
  if command -v readlink >/dev/null 2>&1 && readlink -f / >/dev/null 2>&1; then
    readlink -f -- "${p}" 2>/dev/null && return 0
  fi
  python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "${p}"
}

resolved_out="$(canonicalize "${OUT_DIR}")"
resolved_repo="$(canonicalize "${REPO_ROOT}")"
resolved_home="$(canonicalize "${HOME}")"

if [[ -z "${resolved_out}" ]]; then
  echo "[debug-agent-prompts] failed to canonicalize --out='${OUT_DIR}'" >&2
  exit 64
fi
case "${resolved_out}" in
  "/" | "${resolved_home}" | "${resolved_repo}")
    echo "[debug-agent-prompts] refusing to wipe --out (resolves to ${resolved_out})" >&2
    exit 64
    ;;
esac

# Use the canonicalized path from here on so every subsequent command
# (rm, mkdir, per-agent dump writes) operates on the same resolved
# target — no symlink window between validation and deletion.
OUT_DIR="${resolved_out}"
rm -rf "${OUT_DIR}"
mkdir -p "${OUT_DIR}"

# Point the binary at the real, currently-logged-in user's workspace so
# onboarding-generated files like `PROFILE.md` appear in the dump. A
# throwaway mktemp workspace would silently hide them.
#
# Resolution order (matches what the desktop app does):
#   1. $OPENHUMAN_WORKSPACE if explicitly exported.
#   2. ~/.openhuman/users/<active_user_id>/workspace, where active_user_id
#      is read from ~/.openhuman/active_user.toml. This is the path the
#      Tauri app writes PROFILE.md into after onboarding.
#   3. ~/.openhuman/workspace as a last-resort fallback for pre-multi-user
#      installs.
#
# The binary auto-seeds SOUL.md/IDENTITY.md/HEARTBEAT.md when they are
# missing, so pointing at the real workspace is read-mostly — the only
# writes are the same self-healing writes that happen on every normal
# agent run.
if [[ -n "${OPENHUMAN_WORKSPACE:-}" ]]; then
  WORKSPACE="${OPENHUMAN_WORKSPACE}"
else
  ACTIVE_USER_TOML="${HOME}/.openhuman/active_user.toml"
  ACTIVE_USER_ID=""
  if [[ -f "${ACTIVE_USER_TOML}" ]]; then
    # Extract `user_id = "…"` without pulling in a TOML parser — the file
    # is one or two lines and the shape is stable (config/ops.rs writes
    # it with a plain quoted string).
    ACTIVE_USER_ID="$(sed -n 's/^[[:space:]]*user_id[[:space:]]*=[[:space:]]*"\([^"]*\)"[[:space:]]*$/\1/p' "${ACTIVE_USER_TOML}" | head -n 1)"
  fi
  if [[ -n "${ACTIVE_USER_ID}" && -d "${HOME}/.openhuman/users/${ACTIVE_USER_ID}/workspace" ]]; then
    WORKSPACE="${HOME}/.openhuman/users/${ACTIVE_USER_ID}/workspace"
  else
    WORKSPACE="${HOME}/.openhuman/workspace"
  fi
fi

if [[ ! -d "${WORKSPACE}" ]]; then
  echo "[debug-agent-prompts] workspace not found: ${WORKSPACE}" >&2
  echo "[debug-agent-prompts] complete onboarding in the app first, or export OPENHUMAN_WORKSPACE=<path>." >&2
  exit 66
fi

if [[ -f "${WORKSPACE}/PROFILE.md" ]]; then
  PROFILE_STATE="present"
else
  PROFILE_STATE="MISSING (onboarding enrichment has not run)"
fi

echo "[debug-agent-prompts] output dir : ${OUT_DIR}" >&2
echo "[debug-agent-prompts] workspace  : ${WORKSPACE}" >&2
echo "[debug-agent-prompts] PROFILE.md : ${PROFILE_STATE}" >&2
echo "[debug-agent-prompts] stub composio: $([[ ${STUB_COMPOSIO} -eq 1 ]] && echo on || echo off)" >&2
echo >&2

# ── Discover agent ids from `agent list --json` ───────────────────────────
# `mapfile` is bash 4+, but macOS ships bash 3 — use a portable
# read-while-IFS loop instead so the script works out of the box on a
# vanilla `/bin/bash`.
AGENT_LIST_JSON="$("${BIN}" agent list --workspace "${WORKSPACE}" --json 2>/dev/null)"
AGENT_IDS=()
while IFS= read -r line; do
  [[ -n "${line}" ]] && AGENT_IDS+=("${line}")
done < <(printf '%s' "${AGENT_LIST_JSON}" | python3 -c '
import json, sys
for entry in json.load(sys.stdin):
    aid = entry.get("id", "")
    # The synthetic `fork` definition replays the parent verbatim and
    # has no standalone prompt — skip it.
    if aid and aid != "fork":
        print(aid)
')

# Every registered agent — orchestrator included. There's no
# "main" alias anymore: the dumper treats the orchestrator as just
# another agent, which keeps the per-agent render pipeline uniform.
TARGETS=("${AGENT_IDS[@]}")

# ── Build common dump-prompt flag list ────────────────────────────────────
DUMP_FLAGS=(--workspace "${WORKSPACE}")
if [[ ${STUB_COMPOSIO} -eq 1 ]]; then
  DUMP_FLAGS+=(--stub-composio)
fi
if [[ ${WITH_TOOLS} -eq 1 ]]; then
  DUMP_FLAGS+=(--with-tools)
fi
if [[ ${#VERBOSE_FLAG[@]} -gt 0 ]]; then
  DUMP_FLAGS+=("${VERBOSE_FLAG[@]}")
fi

# ── Dump every target ─────────────────────────────────────────────────────
INDEX=0
SUMMARY=""
for AGENT in "${TARGETS[@]}"; do
  INDEX=$((INDEX + 1))
  SAFE_NAME="$(printf '%s' "${AGENT}" | tr -c 'A-Za-z0-9._-' '_')"
  PROMPT_PATH="${OUT_DIR}/${INDEX}_${SAFE_NAME}.md"
  META_PATH="${OUT_DIR}/${INDEX}_${SAFE_NAME}.meta.txt"

  printf '[debug-agent-prompts] %-20s → %s\n' "${AGENT}" "${PROMPT_PATH}" >&2
  if "${BIN}" agent dump-prompt --agent "${AGENT}" "${DUMP_FLAGS[@]}" \
        > "${PROMPT_PATH}" 2> "${META_PATH}"; then
    LINES="$(wc -l < "${PROMPT_PATH}" | tr -d ' ')"
    TOOL_COUNT="$(grep -E '^tool_count:' "${META_PATH}" | awk '{print $2}')"
    SKILL_COUNT="$(grep -E '^skill_tools:' "${META_PATH}" | awk '{print $2}')"
    SUMMARY+="$(printf '%-20s lines=%-5s tools=%-4s skill=%-4s\n' \
        "${AGENT}" "${LINES}" "${TOOL_COUNT:-?}" "${SKILL_COUNT:-?}")
"
  else
    echo "[debug-agent-prompts]   ✘ failed to dump ${AGENT} (see ${META_PATH})" >&2
    SUMMARY+="$(printf '%-20s FAILED — see %s\n' "${AGENT}" "${META_PATH}")
"
  fi
done

# ── Write a summary index file alongside the dumps ────────────────────────
SUMMARY_PATH="${OUT_DIR}/SUMMARY.txt"
{
  echo "OpenHuman agent prompt dump summary"
  echo "Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "Workspace: ${WORKSPACE}"
  echo "Stub composio: $([[ ${STUB_COMPOSIO} -eq 1 ]] && echo on || echo off)"
  echo
  echo "${SUMMARY}"
} > "${SUMMARY_PATH}"

echo >&2
echo "[debug-agent-prompts] done — ${INDEX} prompts dumped" >&2
echo "[debug-agent-prompts] summary  : ${SUMMARY_PATH}" >&2
