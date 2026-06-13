#!/usr/bin/env bash
#
# Local equivalent of the CI shard matrix — runs each suite group as a
# separate fresh WDIO session, matching `.github/workflows/e2e-reusable.yml`'s
# `e2e-linux-full` matrix. Mirroring CI exactly is the only way to reproduce
# CI failures locally: a single shared session that runs all 87 specs hits
# CEF/esbuild instability after ~30 specs.
#
# Usage (from repo root, inside the openhuman_ci Docker container):
#   bash app/scripts/e2e-run-shards.sh
#
# Or via docker-compose (from the host):
#   docker compose -f e2e/docker-compose.yml run --rm e2e \
#     bash -lc "bash app/scripts/e2e-run-shards.sh"
#
# Shards mirror the CI matrix in .github/workflows/e2e-reusable.yml:
#   foundation   = auth, navigation, system
#   chat         = chat, skills, journeys
#   integrations = providers, webhooks, notifications
#   connectors   = connectors
#   commerce     = payments, settings
#
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

# Same matrix as e2e-reusable.yml.
SHARDS=(
  "foundation:auth,navigation,system"
  "chat:chat,skills,journeys"
  "providers:providers,notifications"
  "webhooks:webhooks"
  "connectors:connectors"
  "commerce:payments,settings"
)

# Allow filtering: `bash e2e-run-shards.sh foundation chat`
if [ "$#" -gt 0 ]; then
  WANT=("$@")
  FILTERED=()
  for shard in "${SHARDS[@]}"; do
    name="${shard%%:*}"
    for w in "${WANT[@]}"; do
      if [ "$name" = "$w" ]; then
        FILTERED+=("$shard")
        break
      fi
    done
  done
  SHARDS=("${FILTERED[@]}")
fi

declare -a RESULTS
overall_status=0

for shard in "${SHARDS[@]}"; do
  name="${shard%%:*}"
  suites="${shard#*:}"
  echo ""
  echo "════════════════════════════════════════════════════════════════"
  echo "  Shard: ${name}   (suites: ${suites})"
  echo "════════════════════════════════════════════════════════════════"

  if bash app/scripts/e2e-run-all-flows.sh --skip-preflight --suite="${suites}"; then
    RESULTS+=("${name}: PASS")
  else
    RESULTS+=("${name}: FAIL")
    overall_status=1
  fi
done

echo ""
echo "════════════════════════════════════════════════════════════════"
echo "  Shard summary"
echo "════════════════════════════════════════════════════════════════"
for r in "${RESULTS[@]}"; do
  printf "  %s\n" "$r"
done
echo ""

exit "$overall_status"
