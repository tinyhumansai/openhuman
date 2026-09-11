#!/usr/bin/env bash
# Coverage-presence gate: fail when a changed Rust source file produced NO
# coverage records at all — i.e. the lane never compiled it, so neither the
# scoped test run nor diff-cover could possibly have verified it.
#
# WHY THIS EXISTS (PR #5593). `src/openhuman/hosting/**` is gated behind a Cargo
# feature that is in neither `[features] default` nor
# `scripts/ci/product-features.txt`, so the coverage lane compiled none of it.
# The scoped libtest filter matched nothing (`running 0 tests … ok`) and
# diff-cover reported "No lines with coverage information in this diff". Both
# read the ABSENCE of data as "nothing to check" rather than "we checked
# nothing", and 1,643 lines — including a 511-line test file — merged green.
#
# WHAT THIS ASSERTS, and deliberately not more: every changed Rust source file
# that should have been compiled appears as an `SF:` record in the lcov. It says
# nothing about how well those lines are covered — `diff-cover --fail-under=80`
# still owns that. Separating the two is what keeps this gate free of false
# positives: "no rows at all" is a build-configuration fact, whereas "too few
# covered lines" is a judgement call that already has an owner.
#
# Usage:
#   assert-coverage-presence.sh <lcov> --files <path>...      # scoped mode
#   assert-coverage-presence.sh <lcov> --all                  # whole-tree mode
#
# Exit: 0 clean · 1 unverified files found · 2 usage/environment error.
set -euo pipefail

ALLOWLIST="${ALLOWLIST:-scripts/ci/coverage-presence-allowlist.txt}"

# Progress line on stdout. Prefixed so the lane's log stays greppable.
log() { echo "[ci][cov-presence] $*"; }

# Usage/environment error: stderr, exit 2. Distinct from exit 1 ("found
# unverified files") so a caller can tell a broken invocation from a real
# finding.
die() {
  echo "[ci][cov-presence] $*" >&2
  exit 2
}

# Source paths whose absence from the lcov is EXPECTED and correct. Each entry
# is a path prefix plus the reason it can never appear, so a future reader can
# tell "excluded on purpose" from "forgotten" — the ambiguity that let #4918 sit.
#
#   src/tui/                       `tui` is default-OFF and deliberately never
#                                  forwarded (INTENTIONALLY_NOT_FORWARDED in
#                                  scripts/lib/feature-forwarding.mjs).
#   src/openhuman/test_support/    `e2e-test-support`; the destructive
#                                  `openhuman.test_reset` RPC must never ship.
#   .../browser/native_backend.rs  `browser-native`, an opt-in dev backend.
UNCOVERED_BY_DESIGN='^(src/tui/|src/openhuman/test_support/|src/openhuman/tools/impl/browser/native_backend\.rs$)'

# Invocation help, printed to stdout for --help and to stderr on a usage error.
usage() {
  cat <<'USAGE'
Usage: assert-coverage-presence.sh <lcov-file> (--all | --files <path>...)
  --all            check every eligible src/**/*.rs in the tree
  --files <path>…  check only the given paths (repo-relative)
USAGE
}

[ "$#" -ge 2 ] || {
  usage >&2
  exit 2
}
LCOV="$1"
shift
[ -f "${LCOV}" ] || die "lcov file not found: ${LCOV}"

MODE=""
declare -a want=()
case "${1:-}" in
  --all)
    MODE=all
    shift
    ;;
  --files)
    MODE=files
    shift
    while [ "$#" -gt 0 ]; do
      want+=("$1")
      shift
    done
    ;;
  -h | --help)
    usage
    exit 0
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac

# ---- the set of files the coverage build actually produced records for -------
#
# `SF:` paths are absolute in CI (/__w/openhuman/openhuman/src/...). Two of them
# also contain `..`, because rustc records the literal path written in a
# `#[path = "../foo.rs"]` attribute rather than a canonical one — today
# `config/schema/load/../load_user_state.rs` and
# `tools/../integrations/test_support.rs`. Both are real, compiled, measured
# files; without normalisation they would be reported unverified. Normalise
# before comparing.
#
# Two forms are recorded for every path, and a file matches either.
#
#   1. the `${PWD}`-relative form, which is what CI produces directly; and
#   2. everything from the last `/src/` onward, which is independent of where
#      the checkout lives.
#
# (2) exists because the recorded prefix and `${PWD}` are not reliably the same
# string. Bash resolves its working directory physically, so a checkout reached
# through a symlink — macOS `$TMPDIR` under `/var -> private/var`, a
# bind-mounted or symlinked CI workspace — records `/var/…` while `${PWD}` says
# `/private/var/…`. Prefix stripping then removes nothing, every path stays
# absolute, nothing matches, and the gate false-fails the ENTIRE diff. That is
# the worst failure this script has, so it does not depend on the prefix.
#
# The last `/src/` rather than the first: a developer checkout at
# `~/src/openhuman/` contains two, and the repo-relative path is the trailing
# one. Unambiguous here because no tracked path under `src/` contains a nested
# `src/` component, and all 1,354 `SF:` records in the reference artifact carry
# the `/src/` marker.
covered_file="$(mktemp)"
trap 'rm -f "${covered_file}"' EXIT
sed -n 's/^SF://p' "${LCOV}" \
  | sed "s#^${PWD}/##" \
  | python3 -c 'import sys, posixpath
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    path = posixpath.normpath(line)
    print(path)
    marker = path.rfind("/src/")
    if marker != -1:
        print(path[marker + 1 :])' \
  | sort -u >"${covered_file}"

# Did the coverage build emit records for this repo-relative path?
# Fixed-string, whole-line: a path containing regex metacharacters cannot match
# the wrong entry.
covered() { grep -Fxq "$1" "${covered_file}"; }

# Is this path recorded in the allowlist as deliberately uncompiled?
# Absent allowlist means nothing is exempt, which is the safe direction.
allowlisted() {
  [ -f "${ALLOWLIST}" ] || return 1
  grep -v '^[[:space:]]*#' "${ALLOWLIST}" 2>/dev/null \
    | grep -v '^[[:space:]]*$' \
    | grep -Fxq "$1"
}

# Does this file declare a function outside a line comment?
#
# ONE awk process, deliberately not `grep -v … | grep -q …`. Under the script's
# `set -o pipefail`, `grep -q` exits at the first match, the upstream `grep -v`
# dies of SIGPIPE, and the pipeline reports 141 — so the file reads as "no fn"
# and is silently skipped. It only bites files long enough for the writer to
# still be going when the reader leaves, i.e. exactly the large files this gate
# most needs to check: it wrongly excluded 299 of 1,377 eligible sources,
# `src/openhuman/hosting/tools.rs` (937 lines) among them.
#
# The pattern avoids `\b` (a GNU extension) so the check behaves identically
# under the BSD grep/awk a contributor runs locally and the GNU one in CI.
has_fn() {
  awk '
    /^[[:space:]]*\/\// { next }
    /(^|[^A-Za-z0-9_])fn[[:space:]]+[A-Za-z_]/ { found = 1; exit }
    END { exit(found ? 0 : 1) }
  ' "$1"
}

# ---- eligibility -------------------------------------------------------------
#
# A path is CHECKED only when every one of these holds. Each exclusion is a
# category for which "no lcov rows" is the correct, expected outcome; a rule
# without them false-fails on 623 of 1,972 files (measured against the
# lcov-rust-core artifact of run 32108672413).
eligible() {
  local f="$1" base
  base="$(basename "${f}")"

  case "${f}" in *.rs) ;; *) return 1 ;; esac  # non-Rust: assets, .md, fixtures
  case "${f}" in src/*) ;; *) return 1 ;; esac # only crate sources
  [ -f "${f}" ] || return 1                    # deleted / renamed-away
  case "${f}" in src/lib.rs | src/main.rs | src/bin/*) return 1 ;; esac
  # Test-only sources. We do not demand coverage OF test code, and a test file
  # only ever appears in the lcov as a side effect of its own execution.
  case "${base}" in *_tests.rs | *_test.rs | tests.rs | test.rs | test_support.rs) return 1 ;; esac
  case "${f}" in */tests/* | */test/*) return 1 ;; esac
  # Facade stubs compile only in the OFF direction of their gate; under the
  # product feature set the real module compiles instead. 13 of these exist.
  [ "${base}" = "stub.rs" ] && return 1
  # Per-OS modules behind #[cfg(target_os)]; CI is Linux.
  case "${base}" in macos.rs | windows.rs) return 1 ;; esac
  echo "${f}" | grep -Eq "${UNCOVERED_BY_DESIGN}" && return 1
  allowlisted "${f}" && return 1
  # No instrumentable code: barrel `mod.rs`, pure type/const modules. 319 files
  # have no `fn` at all and can never produce a coverage region.
  has_fn "${f}" || return 1
  return 0
}

declare -a candidates=()
if [ "${MODE}" = all ]; then
  # `git ls-files` is not reliably available here, and its failure mode is
  # silent. In this repo's CI container it prints
  #
  #   fatal: detected dubious ownership in repository at '/__w/openhuman/openhuman'
  #
  # because actions/checkout registers `safe.directory` under a temporarily
  # overridden `HOME` that later steps do not run with. Read through a process
  # substitution that produced an EMPTY candidate list, and this gate then
  # reported "clean — every eligible changed source file produced coverage
  # records" having checked ZERO files: the exact verified-nothing fail-open it
  # exists to close, reproduced inside the fix for it. Observed on run
  # 32367545922.
  #
  # So: take git's answer only if git actually succeeded AND returned something,
  # otherwise walk the filesystem. The eligibility filter is identical either
  # way; the only difference is that an untracked `src/**.rs` in a developer's
  # working tree would also be checked, which is harmless (it is a real file
  # that either compiled or did not).
  listing=""
  if listing="$(git ls-files 'src/*.rs' 'src/**/*.rs' 2>/dev/null)" && [ -n "${listing}" ]; then
    log "enumerating tracked sources with git ls-files"
  else
    log "git ls-files unavailable or empty — falling back to a filesystem walk"
    listing="$(find src -type f -name '*.rs' 2>/dev/null || true)"
  fi
  while IFS= read -r f; do
    [ -n "${f}" ] && candidates+=("${f}")
  done < <(printf '%s\n' "${listing}" | sort -u)
else
  candidates=("${want[@]+"${want[@]}"}")
fi

declare -a unverified=()
checked=0
for f in "${candidates[@]+"${candidates[@]}"}"; do
  eligible "${f}" || continue
  checked=$((checked + 1))
  covered "${f}" || unverified+=("${f}")
done

log "checked ${checked} eligible source file(s) against $(wc -l <"${covered_file}" | tr -d ' ') covered path(s)"

# A whole-tree run that checked nothing has not passed — it has failed to look.
# This repository always contains eligible Rust sources, so zero means the tree
# walk broke, and reporting success on it is the fail-open this gate exists to
# close. `--files` is exempt: a PR touching only tests or docs legitimately has
# nothing to check.
if [ "${MODE}" = all ] && [ "${checked}" -eq 0 ]; then
  die "--all checked 0 eligible files. The tree walk found nothing, which for this repository means it failed rather than that there is nothing to verify. Refusing to report success."
fi

if [ "${#unverified[@]}" -eq 0 ]; then
  log "clean — every eligible changed source file produced coverage records"
  exit 0
fi

echo "::error::Coverage lane produced NO records for ${#unverified[@]} changed source file(s) — they were never compiled, so nothing verified them."
for f in "${unverified[@]}"; do
  echo "::error file=${f}::${f} produced no coverage records. The coverage lane compiles 'default + scripts/ci/product-features.txt'; if this file sits behind a Cargo feature in neither list it was never built. Fix by adding the gate to product-features.txt (and the shell forwarding list), or record it in scripts/ci/coverage-presence-allowlist.txt with a reason."
done
exit 1
