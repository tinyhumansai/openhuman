#!/bin/bash -p
set -euo pipefail

SCRIPT_RELPATH="scripts/tests/test-m224-core-registries-e2e-contract.sh"
RUNNER_RELPATH="scripts/run-m224-core-registries-e2e.sh"
PROXY_RELPATH="scripts/fixtures/m224_registry_capture_proxy.mjs"
FIXTURE_RELPATH="app/test/e2e/fixtures/m224_registry_fixture.sql"
SPEC_RELPATH="app/test/e2e/specs/core-registries-flow.spec.ts"
HELPER_RELPATH="app/test/e2e/helpers/core-registries.ts"
EXPECTED_CORE_SHA="7515ba2796239311dab1381836184d188c498e5b"

SELF_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
ROOT="$SELF_PATH"
if [[ ! -f "$ROOT/$SCRIPT_RELPATH" ]]; then
  echo "ERROR: executing harness is not the canonical repository path" >&2
  exit 2
fi

require_file() {
  local relpath="$1"
  if [[ ! -f "$ROOT/$relpath" || -L "$ROOT/$relpath" ]]; then
    printf 'ERROR: missing required Task6 file: %s\n' "$relpath" >&2
    exit 1
  fi
}

assert_contains() {
  local path="$1"
  local needle="$2"
  if ! grep -Fq "$needle" "$path"; then
    printf 'ERROR: expected %s to contain: %s\n' "$path" "$needle" >&2
    return 1
  fi
}

assert_not_contains() {
  local path="$1"
  local needle="$2"
  if grep -Fq "$needle" "$path"; then
    printf 'ERROR: expected %s to reject: %s\n' "$path" "$needle" >&2
    return 1
  fi
}

assert_line_order() {
  local path="$1"
  local first="$2"
  local second="$3"
  local first_line
  local second_line
  first_line="$(grep -Fn "$first" "$path" | head -n 1 | cut -d: -f1)"
  second_line="$(grep -Fn "$second" "$path" | head -n 1 | cut -d: -f1)"
  if [[ -z "$first_line" || -z "$second_line" || "$first_line" -ge "$second_line" ]]; then
    printf 'ERROR: expected %s to order "%s" before "%s"\n' "$path" "$first" "$second" >&2
    return 1
  fi
}

mutate_copy() {
  local source="$1"
  local expected_count="$2"
  local needle="$3"
  local replacement="$4"
  local dest="$5"
  /usr/bin/python3 - "$source" "$expected_count" "$needle" "$replacement" "$dest" <<'PY'
from pathlib import Path
import sys

source = Path(sys.argv[1]).read_text()
expected_count = int(sys.argv[2])
needle = sys.argv[3]
replacement = sys.argv[4]
dest = Path(sys.argv[5])
count = source.count(needle)
if count != expected_count:
    raise SystemExit(f"expected {expected_count} mutation target for {needle!r}, found {count}")
dest.write_text(source.replace(needle, replacement))
PY
}

validate_runner_source() {
  local runner_path="$1"
  assert_contains "$runner_path" "#!/bin/bash -p" || return 1
  assert_contains "$runner_path" "set -euo pipefail" || return 1
  assert_contains "$runner_path" "set -o pipefail" || return 1
  assert_contains "$runner_path" "export GIT_NO_REPLACE_OBJECTS=1" || return 1
  assert_contains "$runner_path" "$EXPECTED_CORE_SHA" || return 1
  assert_contains "$runner_path" 'OPENHUMAN_SHA="$(git -C "$OPENHUMAN_DIR" rev-parse HEAD)"' || return 1
  assert_contains "$runner_path" 'git -C "$OPENHUMAN_DIR" status --short' || return 1
  assert_contains "$runner_path" 'git -C "$OPENHUMAN_DIR" replace -l' || return 1
  assert_contains "$runner_path" 'git -C "$OPENHUMAN_DIR" ls-files -v' || return 1
  assert_contains "$runner_path" "trap cleanup EXIT" || return 1
  assert_contains "$runner_path" "initdb" || return 1
  assert_contains "$runner_path" 'initdb -U postgres -D "$PG_DATA"' || return 1
  assert_contains "$runner_path" "pg_ctl" || return 1
  assert_contains "$runner_path" "psql" || return 1
  assert_contains "$runner_path" "$PROXY_RELPATH" || return 1
  assert_contains "$runner_path" "$FIXTURE_RELPATH" || return 1
  assert_contains "$runner_path" "0014_connector_registry.sql" || return 1
  assert_contains "$runner_path" 'for migration in "$CORE_DIR"/migrations/00??_*.sql' || return 1
  assert_contains "$runner_path" '[[ "$applied_count" -eq 14 ]]' || return 1
  assert_not_contains "$runner_path" '00{01..09}_*.sql' || return 1
  assert_contains "$runner_path" "audit_logs" || return 1
  assert_contains "$runner_path" "status --short" || return 1
  assert_contains "$runner_path" "/private/tmp/*|/tmp/*|/private/var/folders/*|/var/folders/*" || return 1
  assert_contains "$runner_path" "next_cursor" || return 1
  assert_contains "$runner_path" "cleanup_ok" || return 1
  assert_contains "$runner_path" '"http://127.0.0.1:$CORE_PORT/healthz"' || return 1
  assert_not_contains "$runner_path" '"http://127.0.0.1:$CORE_PORT/health"' || return 1
  assert_contains "$runner_path" "write_meta 0" || return 1
  assert_contains "$runner_path" 'write_meta "$cleanup_ok"' || return 1
  assert_contains "$runner_path" '"openhumanSha": sys.argv[3]' || return 1
  assert_not_contains "$runner_path" '"openhumanSha": "' || return 1
  assert_contains "$runner_path" "\"cleanup_ok\": sys.argv[9] == \"1\"" || return 1
  assert_not_contains "$runner_path" "\"cleanup_ok\": True" || return 1
  assert_contains "$runner_path" "artifact_dir_retained_after_cleanup" || return 1
  assert_contains "$runner_path" 'REAL_CARGO_HOME="${CARGO_HOME:-$REAL_HOME/.cargo}"' || return 1
  assert_contains "$runner_path" 'REAL_RUSTUP_HOME="${RUSTUP_HOME:-$REAL_HOME/.rustup}"' || return 1
  assert_contains "$runner_path" 'CARGO_HOME="$REAL_CARGO_HOME"' || return 1
  assert_contains "$runner_path" 'RUSTUP_HOME="$REAL_RUSTUP_HOME"' || return 1
  assert_contains "$runner_path" 'PATH="$REAL_CARGO_HOME/bin:$PATH"' || return 1
  assert_contains "$runner_path" 'CEF_CDP_PORT="${M224_CEF_CDP_PORT:-19222}"' || return 1
  assert_contains "$runner_path" '[[ "$CEF_CDP_PORT" == "19222" ]]' || return 1
  assert_contains "$runner_path" 'require_free_port "$CEF_CDP_PORT" "CEF CDP"' || return 1
  assert_contains "$runner_path" "collect_process_tree" || return 1
  assert_contains "$runner_path" 'stop_process_tree "$CORE_PID"' || return 1
  assert_contains "$runner_path" 'port_is_closed "$CORE_PORT"' || return 1
  assert_contains "$runner_path" 'port_is_closed "$PROXY_PORT"' || return 1
  assert_contains "$runner_path" 'port_is_closed "$CEF_CDP_PORT"' || return 1
  assert_contains "$runner_path" 'port_is_closed "$APPIUM_PORT"' || return 1
  assert_line_order "$runner_path" 'rm -rf "$RUN_ROOT"' 'write_meta "$cleanup_ok"' || return 1
  assert_contains "$runner_path" "cmp_snapshots" || return 1
  assert_contains "$runner_path" "cursorPresent" || return 1
  assert_contains "$runner_path" 'if set(entry) - {"method", "path", "statusCode", "blocked", "cursorPresent"}:' || return 1
  assert_contains "$runner_path" 'if not isinstance(entry["cursorPresent"], bool):' || return 1
  assert_contains "$runner_path" "paged_base_paths = (" || return 1
  assert_contains "$runner_path" '"/api/v1/kernel/agents",' || return 1
  assert_contains "$runner_path" '"/api/v1/kernel/tool-definitions",' || return 1
  assert_contains "$runner_path" '"/api/v1/kernel/connector-types",' || return 1
  assert_contains "$runner_path" '"/api/v1/kernel/connector-bindings",' || return 1
  assert_contains "$runner_path" 'cursor_states_by_path = {base_path: set() for base_path in paged_base_paths}' || return 1
  assert_contains "$runner_path" 'cursor_states_by_path[base_path].add(entry["cursorPresent"])' || return 1
  assert_contains "$runner_path" "if False not in cursor_states:" || return 1
  assert_contains "$runner_path" "if True not in cursor_states:" || return 1
  assert_contains "$runner_path" 'tool_enablement_paths = {' || return 1
  assert_contains "$runner_path" 'if entry["path"] in tool_enablement_paths and entry["cursorPresent"]:' || return 1
  assert_not_contains "$runner_path" "rm -rf /" || return 1
}

validate_proxy_source() {
  local proxy_path="$1"
  assert_contains "$proxy_path" "ALLOWED_GET_PATTERNS" || return 1
  assert_contains "$proxy_path" "PAGED_QUERY_KEYS" || return 1
  assert_contains "$proxy_path" "new Set(['limit', 'cursor'])" || return 1
  assert_contains "$proxy_path" "validatePagedQuery" || return 1
  assert_contains "$proxy_path" "rawUrl.startsWith('/')" || return 1
  assert_contains "$proxy_path" "requestUrl.origin === targetOrigin" || return 1
  assert_contains "$proxy_path" "searchParams.entries()" || return 1
  assert_contains "$proxy_path" "seenKeys" || return 1
  assert_contains "$proxy_path" "rawPairs.some(segment => segment.length === 0)" || return 1
  assert_contains "$proxy_path" "return seenKeys.size === rawPairs.length;" || return 1
  assert_contains "$proxy_path" "requestUrl.search.length === 0" || return 1
  assert_contains "$proxy_path" "limit" || return 1
  assert_contains "$proxy_path" "authorization" || return 1
  assert_contains "$proxy_path" "cursorPresent" || return 1
  assert_contains "$proxy_path" "const cursorPresent = parsed.searchParams.has('cursor');" || return 1
  assert_line_order "$proxy_path" "const cursorPresent = parsed.searchParams.has('cursor');" "safe.searchParams.delete('cursor')" || return 1
  assert_contains "$proxy_path" "safe.searchParams.delete('cursor')" || return 1
  assert_not_contains "$proxy_path" "safe.searchParams.set('cursor', '[redacted]')" || return 1
  assert_not_contains "$proxy_path" "searchParams.has('cursor=')" || return 1
  assert_contains "$proxy_path" "statusCode" || return 1
  assert_contains "$proxy_path" "method" || return 1
  assert_contains "$proxy_path" "path" || return 1
  assert_contains "$proxy_path" "GET" || return 1
  assert_not_contains "$proxy_path" "body:" || return 1
}

validate_fixture_source() {
  local fixture_path="$1"
  assert_contains "$fixture_path" "INSERT INTO kernel_tenants" || return 1
  assert_contains "$fixture_path" "INSERT INTO kernel_agents" || return 1
  assert_contains "$fixture_path" "format('20000000-0000-4000-8000-%012s', lpad((n + 100)::text, 12, '0'))::uuid" || return 1
  assert_contains "$fixture_path" "INSERT INTO kernel_tool_definitions" || return 1
  assert_contains "$fixture_path" "INSERT INTO kernel_tool_enablements" || return 1
  assert_contains "$fixture_path" "INSERT INTO kernel_connector_types" || return 1
  assert_contains "$fixture_path" "INSERT INTO kernel_connector_bindings" || return 1
  assert_contains "$fixture_path" "generate_series(1, 52)" || return 1
  assert_contains "$fixture_path" "credential://registry/primary" || return 1
  assert_not_contains "$fixture_path" "sk-live" || return 1
  assert_not_contains "$fixture_path" "Bearer " || return 1
}

require_file "$HELPER_RELPATH"
require_file "$SPEC_RELPATH"
require_file "$RUNNER_RELPATH"
require_file "$PROXY_RELPATH"
require_file "$FIXTURE_RELPATH"

assert_contains "$ROOT/$HELPER_RELPATH" "openCoreRegistriesFromHome"
assert_contains "$ROOT/$SPEC_RELPATH" "walks the Core registries route through exact links"

validate_runner_source "$ROOT/$RUNNER_RELPATH"
validate_proxy_source "$ROOT/$PROXY_RELPATH" || exit 1
validate_fixture_source "$ROOT/$FIXTURE_RELPATH" || exit 1

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/m224-core-registries-contract.XXXXXX")"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

mutate_copy \
  "$ROOT/$RUNNER_RELPATH" \
  1 \
  "$EXPECTED_CORE_SHA" \
  "0000000000000000000000000000000000000000" \
  "$TMP_DIR/runner-wrong-sha.sh"
if validate_runner_source "$TMP_DIR/runner-wrong-sha.sh" 2>/dev/null; then
  echo "ERROR: wrong Core SHA mutation did not fail closed" >&2
  exit 1
fi

mutate_copy \
  "$ROOT/$RUNNER_RELPATH" \
  1 \
  'OPENHUMAN_SHA="$(git -C "$OPENHUMAN_DIR" rev-parse HEAD)"' \
  'OPENHUMAN_SHA="0000000000000000000000000000000000000000"' \
  "$TMP_DIR/runner-hardcoded-openhuman-sha.sh"
if validate_runner_source "$TMP_DIR/runner-hardcoded-openhuman-sha.sh" 2>/dev/null; then
  echo "ERROR: hardcoded OpenHuman SHA mutation did not fail closed" >&2
  exit 1
fi

mutate_copy \
  "$ROOT/$RUNNER_RELPATH" \
  1 \
  'stop_process_tree "$CORE_PID"' \
  'kill "$CORE_PID"' \
  "$TMP_DIR/runner-core-child-leak.sh"
if validate_runner_source "$TMP_DIR/runner-core-child-leak.sh" 2>/dev/null; then
  echo "ERROR: Core child-process cleanup mutation did not fail closed" >&2
  exit 1
fi

mutate_copy \
  "$ROOT/$RUNNER_RELPATH" \
  1 \
  '"http://127.0.0.1:$CORE_PORT/healthz"' \
  '"http://127.0.0.1:$CORE_PORT/health"' \
  "$TMP_DIR/runner-wrong-health.sh"
if validate_runner_source "$TMP_DIR/runner-wrong-health.sh" 2>/dev/null; then
  echo "ERROR: /healthz contract mutation did not fail closed" >&2
  exit 1
fi

mutate_copy \
  "$ROOT/$RUNNER_RELPATH" \
  1 \
  "trap cleanup EXIT" \
  "# cleanup trap removed" \
  "$TMP_DIR/runner-no-trap.sh"
if validate_runner_source "$TMP_DIR/runner-no-trap.sh" 2>/dev/null; then
  echo "ERROR: cleanup trap mutation did not fail closed" >&2
  exit 1
fi

mutate_copy \
  "$ROOT/$RUNNER_RELPATH" \
  2 \
  "cmp_snapshots" \
  "pretend_equal" \
  "$TMP_DIR/runner-no-equivalence.sh"
if validate_runner_source "$TMP_DIR/runner-no-equivalence.sh" 2>/dev/null; then
  echo "ERROR: snapshot equivalence mutation did not fail closed" >&2
  exit 1
fi

mutate_copy \
  "$ROOT/$RUNNER_RELPATH" \
  1 \
  "\"cleanup_ok\": sys.argv[9] == \"1\"" \
  "\"cleanup_ok\": True" \
  "$TMP_DIR/runner-hardcoded-cleanup.sh"
if validate_runner_source "$TMP_DIR/runner-hardcoded-cleanup.sh" 2>/dev/null; then
  echo "ERROR: hardcoded cleanup_ok mutation did not fail closed" >&2
  exit 1
fi

mutate_copy \
  "$ROOT/$RUNNER_RELPATH" \
  1 \
  'if True not in cursor_states:' \
  'if True in cursor_states:' \
  "$TMP_DIR/runner-no-follow-up-cursor-proof.sh"
if validate_runner_source "$TMP_DIR/runner-no-follow-up-cursor-proof.sh" 2>/dev/null; then
  echo "ERROR: follow-up cursor proof mutation did not fail closed" >&2
  exit 1
fi

mutate_copy \
  "$ROOT/$PROXY_RELPATH" \
  2 \
  "ALLOWED_GET_PATTERNS" \
  "BLOCKED_GET_PATTERNS" \
  "$TMP_DIR/proxy-no-allowlist.mjs"
if validate_proxy_source "$TMP_DIR/proxy-no-allowlist.mjs" 2>/dev/null; then
  echo "ERROR: proxy allowlist mutation did not fail closed" >&2
  exit 1
fi

mutate_copy \
  "$ROOT/$PROXY_RELPATH" \
  1 \
  "new Set(['limit', 'cursor'])" \
  "new Set(['limit', 'cursor', 'foo'])" \
  "$TMP_DIR/proxy-extra-query-key.mjs"
if validate_proxy_source "$TMP_DIR/proxy-extra-query-key.mjs" 2>/dev/null; then
  echo "ERROR: proxy paged-query key mutation did not fail closed" >&2
  exit 1
fi

mutate_copy \
  "$ROOT/$PROXY_RELPATH" \
  1 \
  "return seenKeys.size === rawPairs.length;" \
  "return true;" \
  "$TMP_DIR/proxy-duplicate-query-pass.mjs"
if validate_proxy_source "$TMP_DIR/proxy-duplicate-query-pass.mjs" 2>/dev/null; then
  echo "ERROR: proxy duplicate-query mutation did not fail closed" >&2
  exit 1
fi

mutate_copy \
  "$ROOT/$PROXY_RELPATH" \
  1 \
  "requestUrl.search.length === 0" \
  "true" \
  "$TMP_DIR/proxy-nonpaged-query-pass.mjs"
if validate_proxy_source "$TMP_DIR/proxy-nonpaged-query-pass.mjs" 2>/dev/null; then
  echo "ERROR: proxy non-paged query mutation did not fail closed" >&2
  exit 1
fi

mutate_copy \
  "$ROOT/$PROXY_RELPATH" \
  1 \
  "requestUrl.origin === targetOrigin" \
  "true" \
  "$TMP_DIR/proxy-foreign-origin-pass.mjs"
if validate_proxy_source "$TMP_DIR/proxy-foreign-origin-pass.mjs" 2>/dev/null; then
  echo "ERROR: proxy foreign-origin mutation did not fail closed" >&2
  exit 1
fi

mutate_copy \
  "$ROOT/$PROXY_RELPATH" \
  1 \
  "const cursorPresent = parsed.searchParams.has('cursor');" \
  "const cursorPresent = false;" \
  "$TMP_DIR/proxy-hardcoded-cursor-present.mjs"
if validate_proxy_source "$TMP_DIR/proxy-hardcoded-cursor-present.mjs" 2>/dev/null; then
  echo "ERROR: hardcoded cursor presence mutation did not fail closed" >&2
  exit 1
fi

echo "M224 core registries contract probes passed"
