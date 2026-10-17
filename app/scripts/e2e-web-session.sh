#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$APP_DIR/.." && pwd)"
cd "$APP_DIR"

RUST_HOST_TRIPLE="${RUST_HOST_TRIPLE:-$(rustc -vV | awk '/^host: / { print $2 }')}"
E2E_WEB_CORE_TARGET_DIR="${E2E_WEB_CORE_TARGET_DIR:-$REPO_ROOT/target/e2e-web-${RUST_HOST_TRIPLE}}"
# shellcheck source=./e2e-ports.sh
source "$SCRIPT_DIR/e2e-ports.sh"
e2e_resolve_ports
# The Playwright specs reach the mock's admin API themselves
# (`MOCK_ADMIN_BASE`), and default to 18473 when this is unset. Without the
# export a session on a derived block served its specs the wrong mock.
export E2E_MOCK_PORT
PW_CORE_RPC_TOKEN="${PW_CORE_RPC_TOKEN:-openhuman-playwright-token}"
PW_CORE_RPC_URL="http://127.0.0.1:${OPENHUMAN_CORE_PORT}/rpc"
PW_BASE_URL="http://127.0.0.1:${E2E_WEB_PORT}"

E2E_WEB_WORKSPACE_EXPLICIT="${OPENHUMAN_WORKSPACE:+1}"
CREATED_TEMP_WORKSPACE=""
if [ -z "$E2E_WEB_WORKSPACE_EXPLICIT" ]; then
  OPENHUMAN_WORKSPACE="$(mktemp -d)"
  CREATED_TEMP_WORKSPACE="$OPENHUMAN_WORKSPACE"
fi
export OPENHUMAN_WORKSPACE
export OPENHUMAN_KEYRING_BACKEND="${OPENHUMAN_KEYRING_BACKEND:-file}"
if [ -n "$E2E_WEB_WORKSPACE_EXPLICIT" ]; then
  if [ -z "${OPENHUMAN_KEYRING_MASTER_KEY:-}" ] &&
    [ -z "${OPENHUMAN_KEYRING_MASTER_KEY_FILE:-}" ] &&
    [ "${OPENHUMAN_E2E_ALLOW_EPHEMERAL_KEY:-}" != "1" ]; then
    echo "ERROR: an explicit OPENHUMAN_WORKSPACE requires OPENHUMAN_KEYRING_MASTER_KEY or OPENHUMAN_KEYRING_MASTER_KEY_FILE." >&2
    exit 2
  fi
  if [ "${OPENHUMAN_E2E_ALLOW_EPHEMERAL_KEY:-}" = "1" ] &&
    [ -z "${OPENHUMAN_KEYRING_MASTER_KEY:-}" ]; then
    unset OPENHUMAN_KEYRING_MASTER_KEY_FILE
  fi
else
  # A disposable workspace must not accidentally inherit a key-file path
  # outside the test tree from the developer or runner environment.
  unset OPENHUMAN_KEYRING_MASTER_KEY_FILE
fi

MOCK_PID=""
CORE_PID=""
WEB_PID=""
CORE_MONITOR_PID=""
PORT_LOCK_DIR="${TMPDIR:-/tmp}/openhuman-e2e-ports-${E2E_MOCK_PORT}-${OPENHUMAN_CORE_PORT}-${E2E_WEB_PORT}.lock"
PORT_LOCK_ACQUIRED=""

cleanup() {
  local status=$?
  set +e
  if [ -n "$WEB_PID" ]; then
    kill "$WEB_PID" 2>/dev/null || true
    wait "$WEB_PID" 2>/dev/null || true
  fi
  if [ -n "$CORE_PID" ]; then
    # The core may have launched acting-tool subprocesses while an E2E case
    # was running. It leads its own process group either way it was started
    # (`setsid` where available, otherwise bash job control — see the launch
    # site), so stop the whole group rather than leaving descendants alive to
    # accumulate across CI shards.
    kill -- "-$CORE_PID" 2>/dev/null || true
    wait "$CORE_PID" 2>/dev/null || true
  fi
  if [ -n "$CORE_MONITOR_PID" ]; then
    kill "$CORE_MONITOR_PID" 2>/dev/null || true
    wait "$CORE_MONITOR_PID" 2>/dev/null || true
  fi
  if [ -n "$MOCK_PID" ]; then
    kill "$MOCK_PID" 2>/dev/null || true
    wait "$MOCK_PID" 2>/dev/null || true
  fi
  if [ -n "$CREATED_TEMP_WORKSPACE" ]; then
    rm -rf "$CREATED_TEMP_WORKSPACE"
  fi
  if [ -n "$PORT_LOCK_ACQUIRED" ]; then
    rmdir "$PORT_LOCK_DIR" 2>/dev/null || true
  fi
  return "$status"
}
trap cleanup EXIT

wait_for_http() {
  local url="$1"
  local name="$2"
  for _ in $(seq 1 90); do
    if curl -fsS "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "ERROR: ${name} did not become ready at ${url}" >&2
  return 1
}

wait_for_rpc_auth() {
  local rpc_url="$1"
  local token="$2"
  for _ in $(seq 1 30); do
    if curl -fsS "$rpc_url" \
      -H 'Content-Type: application/json' \
      -H "Authorization: Bearer $token" \
      -d '{"jsonrpc":"2.0","id":1,"method":"core.ping","params":{}}' >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "ERROR: authenticated RPC probe failed for ${rpc_url}" >&2
  return 1
}

check_process_alive() {
  local pid="$1"
  local name="$2"
  if ! kill -0 "$pid" 2>/dev/null; then
    echo "ERROR: ${name} (PID ${pid}) has crashed or exited unexpectedly" >&2
    return 1
  fi
  return 0
}

mkdir -p "$OPENHUMAN_WORKSPACE"
# `OPENHUMAN_WORKSPACE` controls the initial config, but authenticated session
# activation deliberately resolves its shared users tree from HOME. Keep that
# tree inside this shard too, otherwise every sign-in in a browser lane mutates
# (and retains services for) the runner's global ~/.openhuman state.
E2E_WEB_CORE_HOME="$OPENHUMAN_WORKSPACE/home"
mkdir -p "$E2E_WEB_CORE_HOME"
# Keep the acting-tool sandbox inside the shard as well. Besides isolating
# browser runs from the host's projects directory, this gives tool-call specs
# a deterministic, readable fixture without touching the checkout.
E2E_ACTION_DIR="$OPENHUMAN_WORKSPACE/action"
mkdir -p "$E2E_ACTION_DIR"
printf 'E2E tool presentation fixture\n' >"$E2E_ACTION_DIR/tool-presentation-fixture.txt"
export OPENHUMAN_ACTION_DIR="$E2E_ACTION_DIR"
cat > "$OPENHUMAN_WORKSPACE/config.toml" <<EOF
api_url = "http://127.0.0.1:${E2E_MOCK_PORT}"
primary_cloud = "p_e2e_mock"
default_model = "e2e-mock-model"
chat_provider = "e2e:e2e-mock-model"
reasoning_provider = "e2e:e2e-mock-model"
agentic_provider = "e2e:e2e-mock-model"
coding_provider = "e2e:e2e-mock-model"
# Tinymemory resolves this workload-routing setting when it initializes its
# binding. Keep it disabled before process startup for the degraded-state
# browser fixture; the memory-section embedding provider alone is not authoritative.
embeddings_provider = "none"
[update]
enabled = false

[memory]
# The browser truthfulness specs seed a real memory source whose chunks must
# remain unembedded. Keep this harness intentionally providerless so it covers
# the hard degraded state rather than a transient backlog being drained.
embedding_provider = "none"

[context]
# Deterministic e2e specs script the mock-LLM call sequence exactly; the

[[cloud_providers]]
id = "p_e2e_mock"
slug = "e2e"
label = "E2E Mock"
endpoint = "http://127.0.0.1:${E2E_MOCK_PORT}/openai/v1"
auth_style = "none"
default_model = "e2e-mock-model"

# The managed backend row. The unify_ai_provider_settings migration seeds this
# on a real install, but its seed_cloud_providers step returns early when
# cloud_providers is already non-empty -- and this fixture ships one entry, so
# the managed row was never seeded here. Without it every managed-source model
# listing fails with: no cloud provider with id or slug 'openhuman' found.
#
# auth_style MUST be spelled openhumanjwt. AuthStyle derives
# serde rename_all = lowercase, so that -- not the openhuman_jwt that
# AuthStyle::as_str returns -- is the wire value. An entry spelled the other
# way fails to deserialize and is dropped from the array in silence.
[[cloud_providers]]
id = "p_e2e_openhuman"
slug = "openhuman"
label = "OpenHuman"
endpoint = "http://127.0.0.1:${E2E_MOCK_PORT}/v1"
auth_style = "openhumanjwt"
EOF

# The bundle must come from `pnpm test:e2e:web:build`, which compiles in the mock
# backend URL and the E2E affordances. A plain `pnpm build:web` exits 0 with a
# bundle this harness cannot drive, and every spec then fails as though the
# product had regressed. Refuse it before starting anything (#5920).
E2E_BUNDLE_MARKER="$APP_DIR/dist-web/openhuman-e2e-bundle.marker"
if [ ! -f "$E2E_BUNDLE_MARKER" ]; then
  echo "ERROR: $APP_DIR/dist-web was not built for E2E (no $(basename "$E2E_BUNDLE_MARKER")). Run pnpm --filter openhuman-app test:e2e:web:build first; pnpm build:web alone omits the E2E backend and affordances." >&2
  exit 1
fi

# Serialize cooperating sessions that selected the same block. Keep the lock
# until cleanup so no second session can pass preflight while these processes
# are starting or running.
if ! mkdir "$PORT_LOCK_DIR" 2>/dev/null; then
  echo "ERROR: another web E2E session is starting or using ports ${E2E_MOCK_PORT},${OPENHUMAN_CORE_PORT},${E2E_WEB_PORT}." >&2
  echo "       Lock: $PORT_LOCK_DIR" >&2
  echo "       Choose a different E2E_PORT_BASE for concurrent sessions." >&2
  exit 1
fi
PORT_LOCK_ACQUIRED=1

# Nothing below tolerates a port that is already taken (#5918): the probes are
# plain HTTP GETs, so they are answered by whatever is listening.
e2e_require_free_ports "$E2E_MOCK_PORT" "$OPENHUMAN_CORE_PORT" "$E2E_WEB_PORT"

node "$REPO_ROOT/scripts/mock-api-server.mjs" --port "$E2E_MOCK_PORT" >"$OPENHUMAN_WORKSPACE/mock.log" 2>&1 &
MOCK_PID=$!
wait_for_http "http://127.0.0.1:${E2E_MOCK_PORT}/__admin/health" "mock backend"

# Refuse a bundle built for different ports (#6478).
#
# `VITE_BACKEND_URL` is substituted into the bundle at BUILD time and has no
# runtime override in web mode, so a bundle built on one `E2E_MOCK_PORT` and
# served against another sends the frontend's own API calls to a mock that is
# not listening — while the core, reading `api_url` from the `config.toml`
# generated below, reaches the right one. Some specs fail and others pass, with
# nothing reporting why. Checked before anything is started so the failure is a
# one-line message rather than a debugging session.
BUILD_PORTS_FILE="$APP_DIR/dist-web/.e2e-build-ports.json"
if [ ! -f "$BUILD_PORTS_FILE" ]; then
  echo "ERROR: $APP_DIR/dist-web has no .e2e-build-ports.json, so the ports it was built for are unknown." >&2
  echo "       It predates this check. Rebuild with: pnpm --filter openhuman-app test:e2e:web:build" >&2
  exit 1
fi
BUILT_MOCK_PORT="$(sed -n 's/.*"e2e_mock_port"[[:space:]]*:[[:space:]]*"\([0-9]*\)".*/\1/p' "$BUILD_PORTS_FILE")"
BUILT_CORE_PORT="$(sed -n 's/.*"openhuman_core_port"[[:space:]]*:[[:space:]]*"\([0-9]*\)".*/\1/p' "$BUILD_PORTS_FILE")"
if [ -z "$BUILT_MOCK_PORT" ]; then
  echo "ERROR: could not read e2e_mock_port from $BUILD_PORTS_FILE. Rebuild with: pnpm --filter openhuman-app test:e2e:web:build" >&2
  exit 1
fi
if [ -z "$BUILT_CORE_PORT" ]; then
  echo "ERROR: could not read openhuman_core_port from $BUILD_PORTS_FILE. Rebuild with: pnpm --filter openhuman-app test:e2e:web:build" >&2
  exit 1
fi
if [ "$BUILT_MOCK_PORT" != "$E2E_MOCK_PORT" ]; then
  echo "ERROR: dist-web was built for E2E_MOCK_PORT=$BUILT_MOCK_PORT but this session uses $E2E_MOCK_PORT." >&2
  echo "       The backend URL is baked into the bundle and cannot be changed at run time," >&2
  echo "       so the app would call a mock that is not listening while the core called the right one." >&2
  echo "       Rebuild with the ports this session uses: E2E_MOCK_PORT=$E2E_MOCK_PORT pnpm --filter openhuman-app test:e2e:web:build" >&2
  exit 1
fi
if [ "$BUILT_CORE_PORT" != "$OPENHUMAN_CORE_PORT" ]; then
  echo "ERROR: dist-web was built for OPENHUMAN_CORE_PORT=$BUILT_CORE_PORT but this session uses $OPENHUMAN_CORE_PORT." >&2
  echo "       Rebuild with the ports this session uses: OPENHUMAN_CORE_PORT=$OPENHUMAN_CORE_PORT pnpm --filter openhuman-app test:e2e:web:build" >&2
  exit 1
fi

# Refuse a bundle older than the sources it was built from (#5919).
#
# The session serves the prebuilt `dist-web`, so editing `app/src` and re-running
# changes nothing in the browser: the spec runs against the PREVIOUS bundle and
# passes, which makes a revert-proof or a fault injection prove nothing at all.
# The marker is written after `build:web` returns, so anything newer than it is a
# source change this bundle does not contain. `-quit` stops at the first hit, so
# this is a stat walk rather than a hash of the tree, and directories are scanned
# too: a file added, removed or renamed after the build shows up as a newer
# directory even when no surviving file is newer.
#
# A missing input is an error rather than a skipped check. `find` on a path that
# does not exist reports nothing, so a renamed input would silently turn this
# gate off and leave the stale bundle served again.
STALE_SOURCE=""
for bundle_input in src public index.html vite.config.ts; do
  input_path="$APP_DIR/$bundle_input"
  if [ ! -e "$input_path" ]; then
    echo "ERROR: bundle input $input_path does not exist, so staleness cannot be checked." >&2
    echo "       Update the input list in $(basename "$0") to match the current layout." >&2
    exit 1
  fi
  if [ -z "$STALE_SOURCE" ]; then
    STALE_SOURCE="$(find "$input_path" -newer "$E2E_BUNDLE_MARKER" -print -quit)"
  fi
done
if [ -n "$STALE_SOURCE" ]; then
  echo "ERROR: $APP_DIR/dist-web is older than the sources it was built from." >&2
  echo "       Newer than the bundle: $STALE_SOURCE" >&2
  echo "       The bundle does not carry that change, so the specs would run against" >&2
  echo "       the previous one and pass." >&2
  echo "       Rebuild first: pnpm --filter openhuman-app test:e2e:web:build" >&2
  exit 1
fi

if [ -z "${OPENHUMAN_KEYRING_MASTER_KEY:-}" ] &&
  [ -z "${OPENHUMAN_KEYRING_MASTER_KEY_FILE:-}" ]; then
  export OPENHUMAN_KEYRING_MASTER_KEY="$(node -e 'process.stdout.write(require("node:crypto").randomBytes(32).toString("hex"))')"
fi

export OPENHUMAN_CORE_BIN="$E2E_WEB_CORE_TARGET_DIR/debug/openhuman-core"
if [ ! -x "$OPENHUMAN_CORE_BIN" ]; then
  echo "ERROR: standalone core binary is missing at $OPENHUMAN_CORE_BIN. Run pnpm --filter openhuman-app test:e2e:web:build first." >&2
  exit 1
fi

export OPENHUMAN_CORE_TOKEN="$PW_CORE_RPC_TOKEN"
# The deterministic browser lane scripts direct tool calls (cron, edits,
# parallel agents, and similar) rather than exercising the model's `use_skill`
# disclosure choreography. Advertise the compiled tool packs for this harness
# core only; production hosts retain the fail-closed packed default.
export OPENHUMAN_E2E=1
# The skills registry defaults to a public HermesHub fetch. The browser E2E
# lane must remain deterministic and offline, so serve its compact catalog
# fixture from the local mock backend instead. The registry's fetch guard
# only allows plain http to loopback with the local-http escape hatch, and
# every SKILL.md is served from the same mock.
export OPENHUMAN_SKILL_REGISTRY_CATALOG_URL="http://127.0.0.1:${E2E_MOCK_PORT}/skills/catalog.json"
export OPENHUMAN_SKILL_REGISTRY_DOWNLOAD_BASE_URL="http://127.0.0.1:${E2E_MOCK_PORT}/skills"
export OPENHUMAN_SKILL_INSTALL_ALLOW_LOCAL_HTTP=1
export OPENHUMAN_TELEGRAM_BOT_API_BASE="http://127.0.0.1:${E2E_MOCK_PORT}"
export OPENHUMAN_COMPOSIO_DIRECT_BASE_V2="http://127.0.0.1:${E2E_MOCK_PORT}"
export OPENHUMAN_COMPOSIO_DIRECT_BASE_V3="http://127.0.0.1:${E2E_MOCK_PORT}"
# Keep the standalone core aligned with the Rust mock runner: sub-agent
# orchestration builds large async futures and can overflow the default stack.
export RUST_MIN_STACK="${RUST_MIN_STACK:-16777216}"

# Give each standalone core its own process group. Playwright shards are run
# serially in CI, and a parent-only shutdown leaves tool children alive across
# shards until the runner terminates the next core for resource exhaustion.
#
# `setsid` is util-linux: it exists on the CI runners and not on macOS, where
# this lane is run locally. Bash's own job control provides the part `cleanup`
# depends on — with `set -m`, a background job becomes the leader of a new
# process group, so `kill -- "-$CORE_PID"` addresses the core and its tool
# children on both platforms and needs no branch of its own. `setsid` is kept
# where it exists so the CI path is unchanged: it also detaches the controlling
# terminal, making the core a full session leader rather than only a process
# group leader.
if command -v setsid >/dev/null 2>&1; then
  env HOME="$E2E_WEB_CORE_HOME" setsid "$OPENHUMAN_CORE_BIN" run --host 127.0.0.1 --port "$OPENHUMAN_CORE_PORT" \
    >"$OPENHUMAN_WORKSPACE/core.log" 2>&1 &
  CORE_PID=$!
else
  set -m
  env HOME="$E2E_WEB_CORE_HOME" "$OPENHUMAN_CORE_BIN" run --host 127.0.0.1 --port "$OPENHUMAN_CORE_PORT" \
    >"$OPENHUMAN_WORKSPACE/core.log" 2>&1 &
  CORE_PID=$!
  set +m
fi

# Preserve the core's final resource samples for a long Playwright lane. This
# distinguishes a likely runner OOM from an in-process failure once later tests
# can only report ECONNREFUSED.
(
  while kill -0 "$CORE_PID" 2>/dev/null; do
    if [ -r "/proc/$CORE_PID/status" ]; then
      awk '/^(VmRSS|VmHWM|Threads):/ { printf "%s ", $0 } END { print "" }' \
        "/proc/$CORE_PID/status" >>"$OPENHUMAN_WORKSPACE/core-resource.log"
    fi
    sleep 5
  done
  printf 'core process disappeared while the Playwright session was active\n' \
    >>"$OPENHUMAN_WORKSPACE/core-resource.log"
) &
CORE_MONITOR_PID=$!

# Give the core process time to start and fail if it's going to
sleep 2
if ! check_process_alive "$CORE_PID" "OpenHuman core"; then
  echo "Core startup failed. Last 50 lines of core.log:" >&2
  tail -50 "$OPENHUMAN_WORKSPACE/core.log" >&2
  exit 1
fi

if ! wait_for_http "http://127.0.0.1:${OPENHUMAN_CORE_PORT}/health" "standalone core"; then
  echo "Core health check failed. Last 50 lines of core.log:" >&2
  tail -50 "$OPENHUMAN_WORKSPACE/core.log" >&2
  exit 1
fi

# The preflight above refuses a port that was already taken, but two sessions
# starting at once can both pass it. The mock and the web host die on
# EADDRINUSE; the core does not. It falls back to preferred+1..+10 and keeps
# running, so `/health` and the RPC probe below would be answered by whoever
# owns the requested port while this core sits on another one. It reports the
# address it actually bound, so ask it rather than trusting the probes (#5918).
CORE_BOUND="$(sed -n 's/.*listening on http:\/\/\([^ ]*\).*/\1/p' \
  "$OPENHUMAN_WORKSPACE/core.log" | tail -n 1)"
if [ -z "$CORE_BOUND" ]; then
  echo "ERROR: could not read the core's bound address from core.log." >&2
  echo "       Refusing to continue without binding evidence." >&2
  exit 1
elif [ "$CORE_BOUND" != "127.0.0.1:${OPENHUMAN_CORE_PORT}" ]; then
  echo "ERROR: the core bound ${CORE_BOUND}, not 127.0.0.1:${OPENHUMAN_CORE_PORT}." >&2
  echo "       Something took that port between the preflight check and startup, so the" >&2
  echo "       core fell back and the health probe was answered by the other listener." >&2
  echo "       Re-run with a free block: E2E_PORT_BASE=<free base>" >&2
  exit 1
fi

if ! wait_for_rpc_auth "$PW_CORE_RPC_URL" "$PW_CORE_RPC_TOKEN"; then
  echo "Core RPC authentication failed. Last 50 lines of core.log:" >&2
  tail -50 "$OPENHUMAN_WORKSPACE/core.log" >&2
  exit 1
fi

python3 -m http.server "$E2E_WEB_PORT" --bind 127.0.0.1 --directory "$APP_DIR/dist-web" \
  >"$OPENHUMAN_WORKSPACE/web.log" 2>&1 &
WEB_PID=$!
wait_for_http "$PW_BASE_URL" "web host"

export PW_BASE_URL
export PW_CORE_RPC_URL
export PW_CORE_RPC_TOKEN

pnpm exec playwright test "$@"
