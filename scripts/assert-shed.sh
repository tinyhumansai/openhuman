#!/usr/bin/env bash
# Assert that crates are absent from a profile's NORMAL dependency graph.
#
# Why not `cargo tree -i <crate>`, which the migration checklist originally
# suggested: two failure modes make it unreliable as a shed proof.
#
#   1. It EXITS NON-ZERO when the crate is absent ("did not match any
#      packages") rather than printing nothing, so a naive `if cargo tree -i`
#      inverts the result.
#   2. `-i` resolves against the whole package set, not the edge kind. A crate
#      that is gone from the normal graph but still present as a
#      dev-dependency is reported as PRESENT even with `-e normal`, because
#      `-e` filters displayed edges, not what `-i` can find. `env_logger` is
#      exactly this case after the `bin-tools` gate.
#
# This checks membership in the same normal-edge graph `kernel-floor.sh`
# counts, so a pass here means the floor metric really did drop.
#
# Usage: scripts/assert-shed.sh <profile> <crate>...
#        scripts/assert-shed.sh tokenjuice-treesitter clap env_logger anstream
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

profile="${1:?usage: assert-shed.sh <profile> <crate>...}"
shift
[[ $# -gt 0 ]] || { echo "assert-shed: no crates given" >&2; exit 2; }

graph="$(GGML_NATIVE=OFF cargo tree -e normal --prefix none \
  --no-default-features --features "$profile" 2>/dev/null \
  | sed 's/ (\*)$//' | awk '{print $1}' | sort -u)"

[[ -n "$graph" ]] || { echo "assert-shed: cargo tree produced nothing for '$profile'" >&2; exit 1; }

status=0
for crate in "$@"; do
  if grep -qx -- "$crate" <<< "$graph"; then
    echo "::error::$crate is STILL in the normal graph of --features $profile"
    status=1
  else
    echo "  $crate: shed"
  fi
done

if (( status == 0 )); then
  echo "all $# crate(s) absent from --features $profile"
fi
exit "$status"
