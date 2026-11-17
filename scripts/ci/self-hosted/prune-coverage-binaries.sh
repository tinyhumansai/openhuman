#!/usr/bin/env bash
# Free the EX63 scratch disk once the core coverage lane has written its lcov.
# Usage: bash scripts/ci/self-hosted/prune-coverage-binaries.sh <coverage-target-dir>
#
# Every instrumented test executable links the whole core (~0.5-1 GB each), and
# the core lane leaves ~90 GB of them behind. Later lanes reuse only the
# dependency rlibs/rmeta and proc-macro .so files in the same target; their own
# test executables are built fresh. Removing the executables and raw profiles
# keeps those artifacts, so nothing is recompiled, and run 39331958941 filled
# the 160 GB disk without it. The profiles go too: lcov-core.info already holds
# them, and a later `cargo llvm-cov report` must not merge profiles whose
# binaries are gone. diff-cover unions ci-out/lcov/*.info, so the result is the
# same.
set -euo pipefail
dir="${1:?usage: prune-coverage-binaries.sh <coverage-target-dir>}"
log() { echo "[ci][prune-coverage] $*"; }
if [ ! -d "${dir}" ]; then
  log "no coverage target at ${dir}; nothing to prune"
  exit 0
fi
before="$(du -sh "${dir}" 2>/dev/null | cut -f1)"
removed=0
while IFS= read -r -d '' file; do
  rm -f -- "${file}"
  removed=$((removed + 1))
done < <(
  # <crate>-<16 hex metadata hash> with no extension: test executables.
  # rlib/rmeta/so/d files and build-script dirs keep their names.
  find "${dir}" -type f -path '*/deps/*' -perm -u+x \
    -name '*-[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]' ! -name '*.*' -print0
)
profiles=0
while IFS= read -r -d '' file; do
  rm -f -- "${file}"
  profiles=$((profiles + 1))
done < <(find "${dir}" -type f \( -name '*.profraw' -o -name '*.profdata' \) -print0)
after="$(du -sh "${dir}" 2>/dev/null | cut -f1)"
log "removed ${removed} test executables and ${profiles} profiles from ${dir}: ${before} -> ${after}"
df -h "${dir}" | tail -n 1 | sed 's/^/[ci][prune-coverage] /'
