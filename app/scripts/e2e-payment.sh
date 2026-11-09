#!/usr/bin/env bash
# Run browser-owned card payment flow tests.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$APP_DIR"
exec bash "$SCRIPT_DIR/e2e-web-session.sh" test/playwright/specs/card-payment-flow.spec.ts
