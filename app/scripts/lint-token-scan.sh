#!/usr/bin/env bash
#
# Fail when a forbidden hardcoded-colour token appears in a scanned path.
#
# WHY THIS IS A SCRIPT AND NOT `! rg ...` IN package.json
# -------------------------------------------------------
# These checks used to be written inline as `! rg <pattern> <paths...>`, relying
# on ripgrep's exit code: 0 = matched, 1 = no match. The negation turned "no
# match" into success.
#
# ripgrep has a THIRD exit code: 2, for an error — most commonly a path that
# does not exist. `! 2` is also 0, so the moment any scanned directory was
# deleted the whole check began reporting success no matter what the remaining
# files contained. It still printed the violations it found; nothing read them.
#
# That is exactly what happened: `src/components/orchestration/` was removed in
# a54c4e7d ("refactor: remove TinyPlace from core and app", #5847) while it was
# still on `lint:ui-tokens`'s path list, and the guard silently stopped guarding.
#
# So: exit codes are handled explicitly here, and anything that is not a clean
# 0/1 from ripgrep is a FAILURE. A guard that cannot look must never report that
# it looked and found nothing.
#
# Usage: lint-token-scan.sh <label> <rg-flags> <pattern> <path>...
set -uo pipefail

if [ "$#" -lt 4 ]; then
  echo "usage: $0 <label> <rg-flags> <pattern> <path>..." >&2
  exit 2
fi

label="$1"
flags="$2"
pattern="$3"
shift 3

if ! command -v rg >/dev/null 2>&1; then
  echo "$label requires ripgrep. Install: brew install ripgrep (macOS) / apt install ripgrep (Debian/Ubuntu) / see https://github.com/BurntSushi/ripgrep#installation" >&2
  exit 1
fi

# Check the paths up front rather than letting ripgrep's error be the signal.
# This names the offending path, which is the thing a maintainer needs: a stale
# entry here means a directory moved or was deleted and the list was not updated.
missing=0
for path in "$@"; do
  if [ ! -e "$path" ]; then
    echo "$label: scanned path does not exist: $path" >&2
    missing=1
  fi
done
if [ "$missing" -ne 0 ]; then
  echo "$label: refusing to report success on an incomplete scan. Update the path list in package.json." >&2
  exit 1
fi

# shellcheck disable=SC2086 # $flags is a deliberate word-split of rg flags.
rg $flags "$pattern" "$@"
status=$?

case "$status" in
  0)
    echo "$label: forbidden token(s) found above." >&2
    exit 1
    ;;
  1)
    # No match — the only success path.
    exit 0
    ;;
  *)
    echo "$label: ripgrep exited $status (not a match/no-match result); treating as a failure rather than a pass." >&2
    exit 1
    ;;
esac
