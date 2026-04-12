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
# Usage:
#   bash scripts/debug-agent-prompts.sh [--out <dir>] [--no-stub-composio] [--with-tools] [-v]
#
# Defaults:
#   --out                  ./prompt-dumps/<UTC timestamp>
#   --stub-composio        ON (override with --no-stub-composio)
#   --with-tools           OFF (pass to also list each agent's tool names)
#

set -euo pipefail

# ── Locate repo root + binary ─────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
BIN="${REPO_ROOT}/target/debug/openhuman-core"

if [[ ! -x "${BIN}" ]]; then
  echo "[debug-agent-prompts] building openhuman-core …" >&2
  ( cd "${REPO_ROOT}" && cargo build --manifest-path Cargo.toml --bin openhuman-core )
fi

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
  TS="$(date -u +%Y%m%dT%H%M%SZ)"
  OUT_DIR="${REPO_ROOT}/prompt-dumps/${TS}"
fi
mkdir -p "${OUT_DIR}"

# Use a throwaway workspace so identity files (`SOUL.md`, `IDENTITY.md`,
# …) get materialised into a tmp dir instead of polluting the user's
# real `~/.openhuman/workspace`.
WORKSPACE="$(mktemp -d -t openhuman-prompt-dump-XXXXXXXX)"
trap 'rm -rf "${WORKSPACE}"' EXIT
export OPENHUMAN_WORKSPACE="${WORKSPACE}"

echo "[debug-agent-prompts] output dir : ${OUT_DIR}" >&2
echo "[debug-agent-prompts] workspace  : ${WORKSPACE}" >&2
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

# Always include the main / orchestrator prompt as the first dump.
TARGETS=("main" "${AGENT_IDS[@]}")

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
