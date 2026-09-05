#!/bin/bash -p
set -euo pipefail
set -o pipefail
export GIT_NO_REPLACE_OBJECTS=1

EXPECTED_CORE_SHA="7515ba2796239311dab1381836184d188c498e5b"
RUNNER_RELPATH="scripts/run-m224-core-registries-e2e.sh"
PROXY_RELPATH="scripts/fixtures/m224_registry_capture_proxy.mjs"
FIXTURE_RELPATH="app/test/e2e/fixtures/m224_registry_fixture.sql"
SPEC_RELPATH="app/test/e2e/specs/core-registries-flow.spec.ts"
OPENHUMAN_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
OPENHUMAN_SHA="$(git -C "$OPENHUMAN_DIR" rev-parse HEAD)"
CORE_DIR_DEFAULT="$(cd "$OPENHUMAN_DIR/.." && pwd -P)/youpet-core"
CORE_DIR="${M224_CORE_DIR:-$CORE_DIR_DEFAULT}"
FIXTURE_PATH="$OPENHUMAN_DIR/$FIXTURE_RELPATH"
PROXY_PATH="$OPENHUMAN_DIR/$PROXY_RELPATH"
SPEC_PATH="$OPENHUMAN_DIR/$SPEC_RELPATH"
OPT_IN="${M224_ALLOW_DISPOSABLE_DB:-0}"

if [[ ! "$OPENHUMAN_SHA" =~ ^[0-9a-f]{40}$ ]]; then
  echo "ERROR: OpenHuman HEAD must resolve to one exact commit" >&2
  exit 2
fi
if [[ -n "$(git -C "$OPENHUMAN_DIR" status --short)" ]]; then
  echo "ERROR: OpenHuman checkout must be clean before live evidence capture" >&2
  exit 2
fi
if [[ -n "$(git -C "$OPENHUMAN_DIR" replace -l)" ]]; then
  echo "ERROR: OpenHuman checkout must not use git replace refs" >&2
  exit 2
fi
if [[ -n "$(git -C "$OPENHUMAN_DIR" ls-files -v | grep -Ev '^H ' || true)" ]]; then
  echo "ERROR: OpenHuman checkout must not use hidden index flags" >&2
  exit 2
fi

if [[ "$OPT_IN" != "1" ]]; then
  echo "ERROR: set M224_ALLOW_DISPOSABLE_DB=1 to authorize the disposable PostgreSQL ceremony" >&2
  exit 2
fi

for required in "$FIXTURE_PATH" "$PROXY_PATH" "$SPEC_PATH"; do
  if [[ ! -f "$required" || -L "$required" ]]; then
    printf 'ERROR: required Task6 artifact is missing: %s\n' "$required" >&2
    exit 2
  fi
done

if [[ ! -d "$CORE_DIR/.git" && ! -f "$CORE_DIR/.git" ]]; then
  printf 'ERROR: exact Core checkout is missing: %s\n' "$CORE_DIR" >&2
  exit 2
fi

if [[ "$(git -C "$CORE_DIR" rev-parse HEAD)" != "$EXPECTED_CORE_SHA" ]]; then
  printf 'ERROR: Core HEAD must stay exact %s\n' "$EXPECTED_CORE_SHA" >&2
  exit 2
fi
if [[ -n "$(git -C "$CORE_DIR" status --short)" ]]; then
  echo "ERROR: Core checkout must be clean/read-only" >&2
  exit 2
fi
if [[ -n "$(git -C "$CORE_DIR" replace -l)" ]]; then
  echo "ERROR: Core checkout must not use git replace refs" >&2
  exit 2
fi

for bin in initdb pg_ctl createdb psql uv pnpm node appium shasum curl python3; do
  if ! command -v "$bin" >/dev/null 2>&1; then
    printf 'ERROR: missing required command: %s\n' "$bin" >&2
    exit 2
  fi
done

pick_free_port() {
  python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

require_free_port() {
  python3 - "$1" "$2" <<'PY'
import socket
import sys
port = int(sys.argv[1])
label = sys.argv[2]
s = socket.socket()
try:
    s.bind(("127.0.0.1", port))
except OSError as exc:
    raise SystemExit(f"ERROR: {label} port {port} is unavailable: {exc}")
finally:
    s.close()
PY
}

require_disposable_path() {
  local path="$1"
  case "$path" in
    /private/tmp/*|/tmp/*|/private/var/folders/*|/var/folders/*) ;;
    *)
      printf 'ERROR: refusing persistent path: %s\n' "$path" >&2
      exit 2
      ;;
  esac
}

RUN_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/m224-core-registries.XXXXXX")"
require_disposable_path "$RUN_ROOT"
PG_DATA="$RUN_ROOT/pg-data"
PG_SOCKET="$RUN_ROOT/pg-socket"
E2E_HOME="$RUN_ROOT/home"
OPENHUMAN_WORKSPACE="$RUN_ROOT/openhuman-workspace"
ARTIFACT_DIR="${M224_ARTIFACT_DIR:-$OPENHUMAN_DIR/app/test/e2e/artifacts/$(date -u +%Y%m%dT%H%M%SZ)-m224-core-registries-live}"
UI_ARTIFACT_DIR="$ARTIFACT_DIR/ui"
PROXY_LOG="$ARTIFACT_DIR/registry-requests.json"
RUNNER_LOG="$ARTIFACT_DIR/runner.log"
CORE_LOG="$RUN_ROOT/youpet-core.log"
DB_NAME="m224_registry_e2e"
CORE_PORT="${M224_CORE_PORT:-$(pick_free_port)}"
PROXY_PORT="${M224_PROXY_PORT:-$(pick_free_port)}"
MOCK_PORT="${M224_MOCK_PORT:-$(pick_free_port)}"
APPIUM_PORT="${M224_APPIUM_PORT:-$(pick_free_port)}"
CEF_CDP_PORT="${M224_CEF_CDP_PORT:-19222}"
PG_PORT=5432
CORE_PID=""
PROXY_PID=""
cleanup_ok=0

[[ "$CEF_CDP_PORT" == "19222" ]] || {
  echo "ERROR: committed CEF runtime requires CDP port 19222" >&2
  exit 2
}
require_free_port "$CEF_CDP_PORT" "CEF CDP"

require_disposable_path "$PG_DATA"
require_disposable_path "$PG_SOCKET"
mkdir -p "$ARTIFACT_DIR" "$UI_ARTIFACT_DIR" "$E2E_HOME" "$OPENHUMAN_WORKSPACE"

REAL_HOME="${HOME}"
REAL_CEF_PATH="${CEF_PATH:-$REAL_HOME/Library/Caches/tauri-cef}"
REAL_NVM_DIR="${NVM_DIR:-$REAL_HOME/.nvm}"
REAL_APPIUM_HOME="${APPIUM_HOME:-$REAL_HOME/.appium}"
REAL_COREPACK_HOME="${COREPACK_HOME:-$REAL_HOME/.cache/node/corepack}"
REAL_CARGO_HOME="${CARGO_HOME:-$REAL_HOME/.cargo}"
REAL_RUSTUP_HOME="${RUSTUP_HOME:-$REAL_HOME/.rustup}"

artifact_dir_retained_after_cleanup() {
  case "$ARTIFACT_DIR" in
    "$RUN_ROOT"|"$RUN_ROOT"/*) return 1 ;;
    *) return 0 ;;
  esac
}

pid_still_running() {
  local pid="$1"
  [[ -n "$pid" ]] && kill -0 "$pid" >/dev/null 2>&1
}

collect_process_tree() {
  local parent_pid="$1"
  local child_pid
  for child_pid in $(pgrep -P "$parent_pid" 2>/dev/null || true); do
    collect_process_tree "$child_pid"
  done
  printf '%s\n' "$parent_pid"
}

stop_process_tree() {
  local root_pid="$1"
  local pid
  local pids
  local still_running
  [[ -n "$root_pid" ]] || return 0
  pids="$(collect_process_tree "$root_pid")"
  for pid in $pids; do
    kill -TERM "$pid" 2>/dev/null || true
  done
  for _ in $(seq 1 30); do
    still_running=0
    for pid in $pids; do
      if kill -0 "$pid" >/dev/null 2>&1; then
        still_running=1
      fi
    done
    [[ "$still_running" -eq 0 ]] && return 0
    sleep 0.1
  done
  for pid in $pids; do
    kill -KILL "$pid" 2>/dev/null || true
  done
  for pid in $pids; do
    kill -0 "$pid" >/dev/null 2>&1 && return 1
  done
  return 0
}

port_is_closed() {
  python3 - "$1" <<'PY'
import socket
import sys
s = socket.socket()
s.settimeout(0.2)
try:
    listening = s.connect_ex(("127.0.0.1", int(sys.argv[1]))) == 0
finally:
    s.close()
raise SystemExit(1 if listening else 0)
PY
}

cleanup() {
  local exit_status=$?
  local retained_artifacts=1
  local process_cleanup_ok=1
  trap - EXIT
  set +e
  if ! artifact_dir_retained_after_cleanup; then
    retained_artifacts=0
  fi

  if pid_still_running "$PROXY_PID"; then
    stop_process_tree "$PROXY_PID" || process_cleanup_ok=0
    wait "$PROXY_PID" 2>/dev/null || true
  fi
  if pid_still_running "$CORE_PID"; then
    stop_process_tree "$CORE_PID" || process_cleanup_ok=0
    wait "$CORE_PID" 2>/dev/null || true
  fi
  if [[ -d "$PG_DATA" ]] && pg_ctl -D "$PG_DATA" status >/dev/null 2>&1; then
    pg_ctl -D "$PG_DATA" -m fast -w stop >/dev/null 2>&1 || true
  fi
  if [[ -d "$RUN_ROOT" ]]; then
    rm -rf "$RUN_ROOT"
  fi
  cleanup_ok="$process_cleanup_ok"
  if pid_still_running "$PROXY_PID" || pid_still_running "$CORE_PID"; then
    cleanup_ok=0
  fi
  if [[ -d "$PG_DATA" ]] && pg_ctl -D "$PG_DATA" status >/dev/null 2>&1; then
    cleanup_ok=0
  fi
  if [[ -e "$RUN_ROOT" ]]; then
    cleanup_ok=0
  fi
  port_is_closed "$CORE_PORT" || cleanup_ok=0
  port_is_closed "$PROXY_PORT" || cleanup_ok=0
  port_is_closed "$CEF_CDP_PORT" || cleanup_ok=0
  port_is_closed "$APPIUM_PORT" || cleanup_ok=0
  if [[ "$retained_artifacts" -eq 1 && -d "$ARTIFACT_DIR" ]]; then
    write_meta "$cleanup_ok"
    write_checksums || cleanup_ok=0
    verify_checksums || cleanup_ok=0
    scan_retained_artifacts || cleanup_ok=0
    if [[ "$cleanup_ok" -eq 0 ]]; then
      write_meta "$cleanup_ok"
      write_checksums || true
    fi
  fi
  if [[ "$cleanup_ok" -eq 0 && "$exit_status" -eq 0 ]]; then
    exit_status=1
  fi
  exit "$exit_status"
}
trap cleanup EXIT

socket_database_url() {
  python3 - "$1" "$2" <<'PY'
from urllib.parse import quote
import sys
socket_dir = quote(sys.argv[1], safe="")
db_name = sys.argv[2]
print(f"postgresql://postgres@/{db_name}?host={socket_dir}&port=5432")
PY
}

DB_URL="$(socket_database_url "$PG_SOCKET" "$DB_NAME")"
export PGHOST="$PG_SOCKET"
export PGPORT="$PG_PORT"
export PGUSER="postgres"

run_psql() {
  psql -X -v ON_ERROR_STOP=1 "$DB_URL" "$@"
}

snapshot_query() {
  local label="$1"
  local query="$2"
  local phase="$3"
  run_psql -At -c "COPY ($query) TO STDOUT" >"$ARTIFACT_DIR/${phase}-${label}.jsonl"
}

snapshot_all() {
  local phase="$1"
  snapshot_query "kernel_tenants" \
    "SELECT row_to_json(t)::text FROM (SELECT id::text AS id, tenant_key, lifecycle_state, created_at::text AS created_at, updated_at::text AS updated_at FROM kernel_tenants ORDER BY tenant_key, id) t" \
    "$phase"
  snapshot_query "kernel_agents" \
    "SELECT row_to_json(t)::text FROM (SELECT id::text AS id, tenant_id::text AS tenant_id, agent_key, version, lifecycle_state, configuration_fingerprint, owner_actor_type, owner_actor_id, created_at::text AS created_at, configuration::text AS configuration FROM kernel_agents ORDER BY tenant_id, agent_key, version, id) t" \
    "$phase"
  snapshot_query "kernel_tool_definitions" \
    "SELECT row_to_json(t)::text FROM (SELECT id::text AS id, tool_key, version, lifecycle_state, definition_fingerprint, schema_version, display_name, description, tool_effect_class, abstract_auth_scopes_json::text AS abstract_auth_scopes_json, input_schema::text AS input_schema, output_schema::text AS output_schema, timeout_defaults_json::text AS timeout_defaults_json, retry_contract_json::text AS retry_contract_json, audit_contract_json::text AS audit_contract_json, created_at::text AS created_at FROM kernel_tool_definitions ORDER BY tool_key, version, id) t" \
    "$phase"
  snapshot_query "kernel_tool_enablements" \
    "SELECT row_to_json(t)::text FROM (SELECT e.id::text AS id, e.tenant_id::text AS tenant_id, e.tool_definition_id::text AS tool_definition_id, d.tool_key, d.version, e.lifecycle_state, e.generation, COALESCE(e.timeout_cap_ms::text, '') AS timeout_cap_ms, e.approval_required, COALESCE(e.allow_ttl_seconds::text, '') AS allow_ttl_seconds, COALESCE(e.audit_mode, '') AS audit_mode, e.created_at::text AS created_at, e.updated_at::text AS updated_at FROM kernel_tool_enablements e JOIN kernel_tool_definitions d ON d.id = e.tool_definition_id ORDER BY e.tenant_id, d.tool_key, d.version, e.id) t" \
    "$phase"
  snapshot_query "kernel_connector_types" \
    "SELECT row_to_json(t)::text FROM (SELECT id::text AS id, connector_key, version, lifecycle_state, source_type, connector_type_fingerprint, capabilities_json::text AS capabilities_json, normalization_contracts_json::text AS normalization_contracts_json, delivery_behavior_json::text AS delivery_behavior_json, created_at::text AS created_at FROM kernel_connector_types ORDER BY connector_key, version, id) t" \
    "$phase"
  snapshot_query "kernel_connector_bindings" \
    "SELECT row_to_json(t)::text FROM (SELECT id::text AS id, tenant_id::text AS tenant_id, binding_key, version, connector_type_id::text AS connector_type_id, connector_type_key, connector_type_version, connector_type_fingerprint, lifecycle_state, provider_namespace, external_account_ref, config_ref, credential_ref, binding_fingerprint, enabled_capabilities_json::text AS enabled_capabilities_json, created_at::text AS created_at FROM kernel_connector_bindings ORDER BY tenant_id, binding_key, version, id) t" \
    "$phase"
  snapshot_query "audit_logs" \
    "SELECT row_to_json(t)::text FROM (SELECT id::text AS id, actor_type::text AS actor_type, COALESCE(actor_id, '') AS actor_id, action, target_type, COALESCE(target_id::text, '') AS target_id, payload_json::text AS payload_json, created_at::text AS created_at FROM audit_logs ORDER BY created_at, id) t" \
    "$phase"
}

cmp_snapshots() {
  local label
  for label in \
    kernel_tenants \
    kernel_agents \
    kernel_tool_definitions \
    kernel_tool_enablements \
    kernel_connector_types \
    kernel_connector_bindings \
    audit_logs; do
    if ! cmp -s "$ARTIFACT_DIR/before-${label}.jsonl" "$ARTIFACT_DIR/after-${label}.jsonl"; then
      printf 'ERROR: registry snapshot mismatch for %s\n' "$label" >&2
      return 1
    fi
  done
}

normalize_ui_sources() {
  python3 - "$UI_ARTIFACT_DIR" <<'PY'
from pathlib import Path
import sys
root = Path(sys.argv[1])
for path in root.rglob("*.source.xml"):
    lines = [line.rstrip() for line in path.read_text().splitlines()]
    path.write_text("\n".join(lines) + ("\n" if lines else ""))
PY
}

verify_proxy_log() {
  python3 - "$PROXY_LOG" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
entries = json.loads(path.read_text() or "[]")
if not entries:
    raise SystemExit("ERROR: proxy log is empty")
expected = {
    "/api/v1/kernel/agents",
    "/api/v1/kernel/agents/agent.registry.001-primary/versions/1",
    "/api/v1/kernel/tool-definitions",
    "/api/v1/kernel/tool-definitions/tool.registry.reader/versions/1",
    "/api/v1/kernel/tool-enablement",
    "/api/v1/kernel/tool-enablement/tool.registry.reader/versions/1",
    "/api/v1/kernel/connector-types",
    "/api/v1/kernel/connector-types/connector.registry.feed/versions/2",
    "/api/v1/kernel/connector-bindings",
    "/api/v1/kernel/connector-bindings/binding.registry-primary/versions/2",
}
paged_base_paths = (
    "/api/v1/kernel/agents",
    "/api/v1/kernel/tool-definitions",
    "/api/v1/kernel/connector-types",
    "/api/v1/kernel/connector-bindings",
)
tool_enablement_paths = {
    "/api/v1/kernel/tool-enablement",
    "/api/v1/kernel/tool-enablement/tool.registry.reader/versions/1",
}
cursor_states_by_path = {base_path: set() for base_path in paged_base_paths}
seen = set()
for entry in entries:
    if set(entry) - {"method", "path", "statusCode", "blocked", "cursorPresent"}:
        raise SystemExit("ERROR: proxy log retained unexpected fields")
    if entry["method"] != "GET":
        raise SystemExit(f"ERROR: non-GET registry bridge request captured: {entry}")
    if not isinstance(entry["cursorPresent"], bool):
        raise SystemExit(f"ERROR: proxy log omitted boolean cursor evidence: {entry}")
    if "cursor=" in entry["path"]:
        raise SystemExit(f"ERROR: cursor leaked into proxy artifact: {entry['path']}")
    if "authorization" in entry["path"].lower():
        raise SystemExit(f"ERROR: authorization leaked into proxy artifact: {entry['path']}")
    if entry.get("blocked"):
        raise SystemExit(f"ERROR: blocked registry bridge request observed: {entry}")
    base_path = entry["path"].split("?", 1)[0]
    if base_path in cursor_states_by_path:
        cursor_states_by_path[base_path].add(entry["cursorPresent"])
    if entry["path"] in tool_enablement_paths and entry["cursorPresent"]:
        raise SystemExit(f"ERROR: unpaged tool enablement request reported cursor usage: {entry}")
    seen.add(base_path)
missing = expected - seen
if missing:
    raise SystemExit(f"ERROR: expected registry paths were not observed: {sorted(missing)}")
for base_path, cursor_states in cursor_states_by_path.items():
    if False not in cursor_states:
        raise SystemExit(f"ERROR: missing initial paged registry request for {base_path}")
    if True not in cursor_states:
        raise SystemExit(f"ERROR: missing follow-up paged registry request for {base_path}")
PY
}

scan_retained_artifacts() {
  local marker
  for marker in \
    "Bearer " \
    "authorization" \
    "m224-registry-token" \
    "cursor=" \
    "raw_secret" \
    "service_token" \
    "secret-value" \
    "resolver-location"; do
    if grep -R -n -F "$marker" "$ARTIFACT_DIR" >/dev/null 2>&1; then
      printf 'ERROR: retained artifact contains forbidden credential marker: %s\n' "$marker" >&2
      return 1
    fi
  done
}

write_checksums() {
  (
    cd "$ARTIFACT_DIR"
    find . -type f ! -name 'SHA256SUMS' -print0 | sort -z | xargs -0 shasum -a 256 >SHA256SUMS
  )
}

verify_checksums() {
  (
    cd "$ARTIFACT_DIR"
    shasum -a 256 -c SHA256SUMS >/dev/null
  )
}

start_postgres() {
  mkdir -p "$PG_DATA" "$PG_SOCKET"
  initdb -U postgres -D "$PG_DATA" >/dev/null
  pg_ctl -D "$PG_DATA" -l "$RUNNER_LOG" -o "-k '$PG_SOCKET' -h ''" -w start >/dev/null
  createdb "$DB_NAME"
}

apply_migrations() {
  local migration
  local applied_count=0
  local last_migration=""
  # The exact frozen Core contains 0001 through 0014. A four-digit glob is
  # portable to macOS Bash 3.2, unlike zero-padded brace sequences.
  for migration in "$CORE_DIR"/migrations/00??_*.sql; do
    [[ -f "$migration" ]] || {
      echo "ERROR: no Core migrations found" >&2
      return 1
    }
    run_psql -f "$migration" >/dev/null
    applied_count=$((applied_count + 1))
    last_migration="${migration##*/}"
  done
  [[ "$applied_count" -eq 14 ]] || {
    printf 'ERROR: expected 14 Core migrations through 0014, applied %s\n' "$applied_count" >&2
    return 1
  }
  [[ "$last_migration" == "0014_connector_registry.sql" ]] || {
    printf 'ERROR: final Core migration was %s, expected 0014_connector_registry.sql\n' \
      "$last_migration" >&2
    return 1
  }
}

start_core() {
  (
    cd "$CORE_DIR"
    YOUPET_DATABASE_URL="$DB_URL" \
    YOUPET_ENV="test" \
    YOUPET_CONSUMER_AUTH='{"openhuman":{"token":"m224-registry-token","actor_id":"registry-reader"}}' \
    uv run uvicorn app.main:app --host 127.0.0.1 --port "$CORE_PORT"
  ) >"$CORE_LOG" 2>&1 &
  CORE_PID=$!
  for _ in $(seq 1 60); do
    if curl -fsS "http://127.0.0.1:$CORE_PORT/healthz" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "ERROR: exact Core did not become healthy" >&2
  return 1
}

start_proxy() {
  M224_PROXY_TARGET="http://127.0.0.1:$CORE_PORT" \
  M224_PROXY_PORT="$PROXY_PORT" \
  M224_PROXY_LOG="$PROXY_LOG" \
  node "$PROXY_PATH" >>"$RUNNER_LOG" 2>&1 &
  PROXY_PID=$!
  for _ in $(seq 1 30); do
    if curl -fsS "http://127.0.0.1:$PROXY_PORT/api/v1/kernel/agents?limit=1" \
      -H 'Authorization: Bearer m224-registry-token' \
      -H 'X-Actor-Id: registry-reader' >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "ERROR: registry proxy did not become healthy" >&2
  return 1
}

run_openhuman_e2e() {
  (
    cd "$OPENHUMAN_DIR/app"
    HOME="$E2E_HOME" \
    NVM_DIR="$REAL_NVM_DIR" \
    APPIUM_HOME="$REAL_APPIUM_HOME" \
    COREPACK_HOME="$REAL_COREPACK_HOME" \
    CARGO_HOME="$REAL_CARGO_HOME" \
    RUSTUP_HOME="$REAL_RUSTUP_HOME" \
    PATH="$REAL_CARGO_HOME/bin:$PATH" \
    CEF_PATH="$REAL_CEF_PATH" \
    OPENHUMAN_WORKSPACE="$OPENHUMAN_WORKSPACE" \
    OPENHUMAN_KEYRING_BACKEND="file" \
    YOUPET_CORE_API_URL="http://127.0.0.1:$PROXY_PORT" \
    YOUPET_SERVICE_TOKEN="m224-registry-token" \
    YOUPET_WORKBENCH_ACTOR_ID="registry-reader" \
    E2E_ARTIFACT_DIR="$UI_ARTIFACT_DIR" \
    E2E_ARTIFACT_LABEL="m224-core-registries" \
    E2E_MOCK_PORT="$MOCK_PORT" \
    APPIUM_PORT="$APPIUM_PORT" \
    CEF_CDP_PORT="$CEF_CDP_PORT" \
    /bin/bash "$OPENHUMAN_DIR/app/scripts/e2e-build.sh"
  )
  (
    cd "$OPENHUMAN_DIR/app"
    HOME="$E2E_HOME" \
    NVM_DIR="$REAL_NVM_DIR" \
    APPIUM_HOME="$REAL_APPIUM_HOME" \
    COREPACK_HOME="$REAL_COREPACK_HOME" \
    CARGO_HOME="$REAL_CARGO_HOME" \
    RUSTUP_HOME="$REAL_RUSTUP_HOME" \
    PATH="$REAL_CARGO_HOME/bin:$PATH" \
    CEF_PATH="$REAL_CEF_PATH" \
    OPENHUMAN_WORKSPACE="$OPENHUMAN_WORKSPACE" \
    OPENHUMAN_KEYRING_BACKEND="file" \
    YOUPET_CORE_API_URL="http://127.0.0.1:$PROXY_PORT" \
    YOUPET_SERVICE_TOKEN="m224-registry-token" \
    YOUPET_WORKBENCH_ACTOR_ID="registry-reader" \
    E2E_ARTIFACT_DIR="$UI_ARTIFACT_DIR" \
    E2E_ARTIFACT_LABEL="m224-core-registries" \
    E2E_MOCK_PORT="$MOCK_PORT" \
    APPIUM_PORT="$APPIUM_PORT" \
    CEF_CDP_PORT="$CEF_CDP_PORT" \
    /bin/bash "$OPENHUMAN_DIR/app/scripts/e2e-run-session.sh" "test/e2e/specs/core-registries-flow.spec.ts" "m224-core-registries"
  )
}

write_meta() {
  python3 - "$ARTIFACT_DIR/meta.json" "$EXPECTED_CORE_SHA" "$OPENHUMAN_SHA" "$CORE_PORT" "$PROXY_PORT" "$MOCK_PORT" "$APPIUM_PORT" "$CEF_CDP_PORT" "${1:-0}" <<'PY'
import json
from pathlib import Path
import sys

path = Path(sys.argv[1])
path.write_text(
    json.dumps(
        {
            "coreSha": sys.argv[2],
            "openhumanSha": sys.argv[3],
            "corePort": int(sys.argv[4]),
            "proxyPort": int(sys.argv[5]),
            "mockPort": int(sys.argv[6]),
            "appiumPort": int(sys.argv[7]),
            "cefCdpPort": int(sys.argv[8]),
            "cleanup_ok": sys.argv[9] == "1",
            "next_cursor": "redacted",
        },
        indent=2,
    )
    + "\n"
)
PY
}

start_postgres
apply_migrations
run_psql -f "$FIXTURE_PATH" >/dev/null
snapshot_all before
start_core
start_proxy
run_openhuman_e2e
snapshot_all after
cmp_snapshots
verify_proxy_log
normalize_ui_sources
write_meta 0
write_checksums
verify_checksums
scan_retained_artifacts
printf 'M224 core registries live proof passed\nartifacts=%s\ncore_sha=%s\nopenhuman_sha=%s\ncleanup_ok=pending-exit-trap\n' \
  "$ARTIFACT_DIR" "$EXPECTED_CORE_SHA" "$OPENHUMAN_SHA"
