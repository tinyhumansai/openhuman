#!/usr/bin/env bash
# sync.sh <pr-number>
# Check out the PR as local branch pr/<num>, merge main in, wire upstream
# tracking + pushRemote to the contributor's fork. No agent invocation.

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$here/lib.sh"

require git gh jq
require_pr_number "${1:-}"
sync_pr "$1"

echo "[review] done. current branch: $(git branch --show-current)"
