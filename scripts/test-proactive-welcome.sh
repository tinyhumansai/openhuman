#!/usr/bin/env bash
#
# End-to-end smoke test for the proactive welcome flow.
#
# 1. Resets `onboarding_completed` + `chat_onboarding_completed` to false
#    in the staging user's config.toml (the path a source-built binary reads).
# 2. Spawns a fresh `openhuman-core` binary on port 7789 with debug logs
#    (non-default port so it doesn't fight a running `tauri dev` on 7788).
# 3. Connects a Socket.IO client that logs every event it receives.
# 4. Calls `openhuman.config_set_onboarding_completed` with value=true.
# 5. Watches the log up to 120s for each checkpoint in the pipeline.
# 6. Reports pass/miss per checkpoint AND whether the socket client got
#    a `proactive_message` event.
#
# Usage: bash scripts/test-proactive-welcome.sh [--keep-flags]

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$REPO_ROOT/target/debug/openhuman-core"
PORT=7789
USER_ID="69d9cb73e61f755583c3671f"
# Source-built binaries default to `.openhuman-staging`. Production
# staged binary reads `.openhuman`. We point at staging here.
CONFIG_ROOT="${OPENHUMAN_CONFIG_ROOT:-$HOME/.openhuman-staging}"
CONFIG_PATH="$CONFIG_ROOT/users/$USER_ID/config.toml"
LOG_FILE="$(mktemp -t openhuman-proactive-welcome-XXXXXX).log"
SIO_LOG="$(mktemp -t openhuman-sio-XXXXXX).log"
SIO_CLIENT_DIR="/tmp/sio-test"
KEEP_FLAGS=0

for arg in "$@"; do
    case "$arg" in
        --keep-flags) KEEP_FLAGS=1 ;;
    esac
done

log() { printf "[test] %s\n" "$*"; }
fail() { printf "[test][FAIL] %s\n" "$*" >&2; exit 1; }

[[ -f "$BIN" ]] || fail "binary not built: $BIN (run: cargo build --bin openhuman-core)"
[[ -f "$CONFIG_PATH" ]] || fail "config not found: $CONFIG_PATH"

# Flip the two onboarding keys to `false` in place, preserving any
# trailing inline comment and whitespace. If a key is missing, append
# a single line at the end of the file — never prepend, because the
# first line is usually a bare top-level assignment like
# `default_model = "..."` and prepending could land inside a section
# header on files laid out differently.
reset_flags() {
    python3 - "$CONFIG_PATH" <<'PY'
import sys, re, pathlib
p = pathlib.Path(sys.argv[1])
text = p.read_text()
# Match: start-of-line, key, optional spaces, =, spaces, true|false,
# optional trailing whitespace + "# comment" (captured so we can keep it).
for key in ("onboarding_completed", "chat_onboarding_completed"):
    pat = re.compile(
        rf'^(?P<indent>[ \t]*){key}[ \t]*=[ \t]*(?:true|false)(?P<tail>[ \t]*(?:#.*)?)$',
        re.M,
    )
    m = pat.search(text)
    if m:
        text = pat.sub(lambda mm: f"{mm.group('indent')}{key} = false{mm.group('tail')}", text, count=1)
    else:
        if not text.endswith("\n"):
            text += "\n"
        text += f"{key} = false\n"
p.write_text(text)
PY
}

# Back up the config before touching it so cleanup can restore the
# user's original state verbatim (including comments, section order,
# and any unrelated fields). Belt-and-suspenders: we still call
# `reset_flags` pre-run to guarantee the two flags are `false` when
# the binary reads them, but the exit-trap uses `mv` of the backup
# so nothing we write survives unless `--keep-flags` is set.
CONFIG_BACKUP="${CONFIG_PATH}.bak.$$"
log "backing up $CONFIG_PATH -> $CONFIG_BACKUP"
cp "$CONFIG_PATH" "$CONFIG_BACKUP"

log "resetting flags in $CONFIG_PATH"
reset_flags
grep -E '^(onboarding_completed|chat_onboarding_completed)\s*=' "$CONFIG_PATH" | sed 's/^/[test][config-before] /'

log "starting $BIN on port $PORT (log: $LOG_FILE)"
# Pre-seed the RPC bearer token so the single curl call below can authenticate.
RPC_TOKEN="$(openssl rand -hex 32 2>/dev/null || python3 -c 'import secrets; print(secrets.token_hex(32))')"
RUST_LOG=debug,hyper=info,tungstenite=info,socketioxide=info \
    OPENHUMAN_CORE_TOKEN="$RPC_TOKEN" \
    "$BIN" run --port "$PORT" > "$LOG_FILE" 2>&1 &
BIN_PID=$!

cleanup() {
    log "cleanup: killing bin pid=$BIN_PID (+ sio pid=${SIO_PID:-none})"
    [[ -n "${SIO_PID:-}" ]] && kill "$SIO_PID" 2>/dev/null || true
    if kill -0 "$BIN_PID" 2>/dev/null; then
        kill "$BIN_PID" 2>/dev/null || true
        wait "$BIN_PID" 2>/dev/null || true
    fi
    # Restore the original config from the backup — runs on both
    # success and failure so the developer's staging profile is never
    # permanently mutated by a test run. `--keep-flags` opts out so
    # the flipped-to-true state survives for interactive debugging.
    if [[ -f "$CONFIG_BACKUP" ]]; then
        if [[ "$KEEP_FLAGS" -eq 0 ]]; then
            log "restoring original config from $CONFIG_BACKUP"
            mv "$CONFIG_BACKUP" "$CONFIG_PATH"
        else
            log "--keep-flags set; leaving backup at $CONFIG_BACKUP and current flag state in place"
        fi
    fi
    log "binary log:  $LOG_FILE"
    log "socket log:  $SIO_LOG"
}
trap cleanup EXIT

log "waiting for core to be ready…"
for _ in $(seq 1 60); do
    grep -q "OpenHuman core is ready" "$LOG_FILE" 2>/dev/null && break
    sleep 0.5
done
grep -q "OpenHuman core is ready" "$LOG_FILE" || {
    tail -40 "$LOG_FILE" | sed 's/^/[test][core-log] /'
    fail "core did not become ready"
}
log "core ready"

# Give registry a moment.
sleep 1

# Spawn Socket.IO listener.
if [[ -f "$SIO_CLIENT_DIR/listen.js" && -d "$SIO_CLIENT_DIR/node_modules/socket.io-client" ]]; then
    log "spawning socket.io listener -> $SIO_LOG"
    (cd "$SIO_CLIENT_DIR" && node listen.js "$PORT" "$SIO_LOG" 150) > /dev/null 2>&1 &
    SIO_PID=$!
    sleep 2
    if grep -q CONNECTED "$SIO_LOG" 2>/dev/null; then
        log "socket.io: $(grep CONNECTED "$SIO_LOG" | head -1)"
    else
        log "socket.io client did not confirm CONNECT; continuing anyway"
    fi
else
    log "socket.io-client not installed at $SIO_CLIENT_DIR — skipping"
    SIO_PID=""
fi

log "POST /rpc openhuman.config_set_onboarding_completed {value:true}"
RPC_RESP=$(curl -s -X POST "http://127.0.0.1:$PORT/rpc" \
    -H 'content-type: application/json' \
    -H "Authorization: Bearer $RPC_TOKEN" \
    -d '{"jsonrpc":"2.0","id":1,"method":"openhuman.config_set_onboarding_completed","params":{"value":true}}')
echo "[test][rpc-response] $RPC_RESP"
echo "$RPC_RESP" | grep -q '"result"' || fail "RPC did not return a result"

log "watching log for welcome pipeline (timeout 120s)…"
CHECK_TRANSITION="[onboarding] false→true transition detected"
CHECK_SPAWN="[welcome::proactive] starting proactive welcome"
CHECK_INVOKE="[welcome::proactive] invoking welcome agent run_single"
CHECK_PRODUCED="[welcome::proactive] welcome agent produced message"
CHECK_PUBLISHED="[proactive] handling proactive message"
CHECK_EMITTED="[socketio] send event=proactive_message"

deadline=$((SECONDS + 120))
while (( SECONDS < deadline )); do
    if grep -qF "$CHECK_PRODUCED" "$LOG_FILE" 2>/dev/null \
       || grep -qE "\[welcome::proactive\] failed to deliver" "$LOG_FILE" 2>/dev/null; then
        break
    fi
    sleep 1
done

log "=== checkpoint summary (backend) ==="
for label in \
    "TRANSITION:$CHECK_TRANSITION" \
    "SPAWN:$CHECK_SPAWN" \
    "INVOKE:$CHECK_INVOKE" \
    "PRODUCED:$CHECK_PRODUCED" \
    "PUBLISHED:$CHECK_PUBLISHED" \
    "EMITTED:$CHECK_EMITTED"; do
    name="${label%%:*}"
    needle="${label#*:}"
    if grep -qF "$needle" "$LOG_FILE" 2>/dev/null; then
        printf "[test][PASS] %-11s %s\n" "$name" "$needle"
    else
        printf "[test][MISS] %-11s %s\n" "$name" "$needle"
    fi
done

# Wait a couple more seconds for the socket event round-trip, then inspect.
sleep 3
log "=== client-side socket.io events ==="
if [[ -f "$SIO_LOG" && -s "$SIO_LOG" ]]; then
    cat "$SIO_LOG" | sed 's/^/[test][sio] /'
    if grep -q 'EVENT proactive_message' "$SIO_LOG" 2>/dev/null \
       || grep -q 'EVENT proactive:message' "$SIO_LOG" 2>/dev/null; then
        printf "[test][PASS] %-11s %s\n" "DELIVERY" "socket.io client received proactive_message"
    else
        printf "[test][MISS] %-11s %s\n" "DELIVERY" "socket.io client did NOT receive proactive_message (server emitted to room=system; clients auto-join only their own sid room)"
    fi
else
    log "no socket.io log (listener not started)"
fi

log "=== welcome agent full message (from log) ==="
python3 - "$LOG_FILE" <<'PY'
import re, pathlib, sys
t = pathlib.Path(sys.argv[1]).read_text()
m = re.search(r'provider response: ChatResponse \{ text: Some\("(.*?)"\)', t, re.S)
if m:
    body = m.group(1).encode('utf-8').decode('unicode_escape')
    print(body)
else:
    print("(no final assistant text found in log)")
PY

echo "[test] done."
