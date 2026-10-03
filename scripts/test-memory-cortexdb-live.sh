#!/usr/bin/env bash
# Memory v2 end to end against a real CortexDB server.
#
# Boots the pinned CortexDB harness from vendor/tinymemory/integration/cortexdb
# in Docker, then runs tests/memory_cortexdb_live.rs: the real `openhuman-core`
# binary with the `cortexdb` engine, driving learnings, a synced folder source,
# web-chat conversation ingestion, recall, and context.md (cron-compiled,
# written to disk, and injected into a new thread's first message).
#
#   scripts/test-memory-cortexdb-live.sh
#   KEEP=1 scripts/test-memory-cortexdb-live.sh          # leave CortexDB running
#   CORTEXDB_VERSION=v0.9.9 scripts/test-memory-cortexdb-live.sh
#
# Needs docker and node on PATH. Nothing reaches the network beyond loopback:
# CortexDB's models are the harness's deterministic inference double and the
# agent's are the shared node mock backend.

set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
harness="$repo/vendor/tinymemory/integration/cortexdb/docker-compose.yml"
compose=(docker compose --project-name openhuman-memory-cortexdb -f "$harness")
port="${CORTEXDB_PORT:-3141}"
url="http://127.0.0.1:$port"
export CORTEXDB_PORT="$port"

if [ ! -f "$harness" ]; then
  echo "missing $harness; run: git submodule update --init --recursive vendor/" >&2
  exit 1
fi

cleanup() {
  result=$?
  if [ "$result" -ne 0 ]; then
    "${compose[@]}" logs cortex | tail -60 || true
  fi
  if [ -z "${KEEP:-}" ]; then
    "${compose[@]}" down --volumes --remove-orphans >/dev/null 2>&1 || true
  fi
  exit "$result"
}
trap cleanup EXIT

"${compose[@]}" up -d --build --wait mock-inference >/dev/null
"${compose[@]}" up -d cortex >/dev/null
for _ in $(seq 1 120); do
  curl --fail --silent "$url/v1/admin/ready" >/dev/null && break
  sleep 1
done
curl --fail --silent "$url/v1/admin/ready" >/dev/null || {
  echo "CortexDB did not become ready at $url" >&2
  exit 1
}
echo "CortexDB $(curl --silent "$url/v1/admin/health") at $url"

cd "$repo"
OPENHUMAN_LIVE_CORTEXDB_URL="$url" \
  cargo test -p openhuman-cli --test memory_cortexdb_live -- --nocapture
