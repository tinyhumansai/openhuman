#!/usr/bin/env bash
# Run E2E auth & access control tests only. See app/scripts/e2e-run-spec.sh.
set -euo pipefail
exec "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/e2e-run-spec.sh" "test/e2e/specs/auth-session-management.spec.ts" "auth"
