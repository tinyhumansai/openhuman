#!/usr/bin/env bash

set -euo pipefail

if [ "$#" -eq 0 ]; then
  echo "usage: $0 <command> [args...]" >&2
  exit 64
fi

OS_NAME="$(uname -s 2>/dev/null || echo unknown)"
CHILD_PID=""
RECEIVED_SIGNAL=""
CHILD_OWNS_PROCESS_GROUP=0

is_windows_shell() {
  case "$OS_NAME" in
    MINGW*|MSYS*|CYGWIN*|Windows_NT) return 0 ;;
    *) return 1 ;;
  esac
}

collect_descendants_unix() {
  local pid="$1"
  local child=""
  while IFS= read -r child; do
    [ -n "$child" ] || continue
    collect_descendants_unix "$child"
    printf '%s\n' "$child"
  done < <(pgrep -P "$pid" 2>/dev/null || true)
}

terminate_tree_term() {
  local pid="$1"
  if is_windows_shell; then
    taskkill //PID "$pid" //T >/dev/null 2>&1 || true
    return
  fi

  if [ "$CHILD_OWNS_PROCESS_GROUP" = "1" ]; then
    kill -TERM -- "-$pid" 2>/dev/null || true
  fi

  local descendants=""
  descendants="$(collect_descendants_unix "$pid")"
  if [ -n "$descendants" ]; then
    while IFS= read -r child; do
      [ -n "$child" ] || continue
      kill -TERM "$child" 2>/dev/null || true
    done <<< "$descendants"
  fi
  kill -TERM "$pid" 2>/dev/null || true
}

terminate_tree_kill() {
  local pid="$1"
  if is_windows_shell; then
    taskkill //PID "$pid" //T //F >/dev/null 2>&1 || true
    return
  fi

  if [ "$CHILD_OWNS_PROCESS_GROUP" = "1" ]; then
    kill -KILL -- "-$pid" 2>/dev/null || true
  fi

  local descendants=""
  descendants="$(collect_descendants_unix "$pid")"
  if [ -n "$descendants" ]; then
    while IFS= read -r child; do
      [ -n "$child" ] || continue
      kill -KILL "$child" 2>/dev/null || true
    done <<< "$descendants"
  fi
  kill -KILL "$pid" 2>/dev/null || true
}

forward_cancel() {
  local signal="$1"
  RECEIVED_SIGNAL="$signal"
  if [ -n "$CHILD_PID" ] && kill -0 "$CHILD_PID" 2>/dev/null; then
    echo "[ci-cancel-aware] received $signal, terminating process tree rooted at $CHILD_PID" >&2
    terminate_tree_term "$CHILD_PID"
  fi
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM HUP

  if [ -n "$CHILD_PID" ] && kill -0 "$CHILD_PID" 2>/dev/null; then
    terminate_tree_term "$CHILD_PID"
    for _ in $(seq 1 10); do
      if ! kill -0 "$CHILD_PID" 2>/dev/null; then
        break
      fi
      sleep 1
    done
    if kill -0 "$CHILD_PID" 2>/dev/null; then
      echo "[ci-cancel-aware] forcing process tree shutdown for pid $CHILD_PID" >&2
      terminate_tree_kill "$CHILD_PID"
    fi
    wait "$CHILD_PID" 2>/dev/null || true
  fi

  if [ -n "$RECEIVED_SIGNAL" ]; then
    case "$RECEIVED_SIGNAL" in
      INT) return 130 ;;
      TERM|HUP) return 143 ;;
      *) return "$status" ;;
    esac
  fi

  return "$status"
}

start_child() {
  if ! is_windows_shell && command -v setsid >/dev/null 2>&1; then
    setsid "$@" &
    CHILD_OWNS_PROCESS_GROUP=1
  else
    "$@" &
    CHILD_OWNS_PROCESS_GROUP=0
  fi
  CHILD_PID=$!
}

trap 'forward_cancel INT' INT
trap 'forward_cancel TERM' TERM
trap 'forward_cancel HUP' HUP
trap cleanup EXIT

echo "[ci-cancel-aware] exec: $(printf '%q ' "$@")" >&2
start_child "$@"

set +e
wait "$CHILD_PID"
status=$?
set -e

# A trapped cancellation can interrupt wait before the child exits. Keep
# CHILD_PID set so the EXIT cleanup can escalate from TERM to KILL.
if ! kill -0 "$CHILD_PID" 2>/dev/null; then
  CHILD_PID=""
fi
exit "$status"
