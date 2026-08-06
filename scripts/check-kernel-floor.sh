#!/usr/bin/env bash
# Enforce the dependency-floor ratchet declared in `scripts/kernel-floor.limits`.
#
# The kernel profile (`--no-default-features --features flows`) is the surface a
# second host would embed. Left unmeasured it grows: openhuman already carries
# seven unconditional heavy dependencies that a reviewer would probably have
# questioned had a number moved when they landed. This lane is that number.
#
# Fails when a profile exceeds its limit, and also when it comes in *under* by
# more than a slack margin without the limit being lowered — because a shed that
# nobody ratchets down is a shed that silently grows back.
#
# Usage: scripts/check-kernel-floor.sh [--verbose]
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

LIMITS="scripts/kernel-floor.limits"
VERBOSE="${1:-}"
# How far under a limit a profile may sit before we insist the limit be lowered.
#
# Was 5, which was too loose to do its job: the M2 gating wave sheds 2, 3, 3, 4
# and 5 names in five of its eight steps, and every one of those would have
# passed CI with a stale limit — i.e. the majority of the work this ratchet
# exists to protect could have silently grown back.
#
# 1 still absorbs the only benign case (an upstream crate splitting or merging
# on its own, which moves the count by one) while forcing a ratchet update for
# any shed of 2 or more.
SLACK=1

status=0

while IFS= read -r line; do
  line="${line%%#*}"
  line="$(echo "$line" | tr -d '[:space:]')"
  [[ -z "$line" ]] && continue

  IFS=: read -r profile max_packages max_names max_native extra <<< "$line"
  if [[ -n "${extra:-}" || -z "$profile" || -z "$max_packages" || \
        -z "$max_names" || -z "$max_native" ]]; then
    echo "::error::invalid kernel-floor limit entry: '$line'" >&2
    exit 1
  fi

  if ! json_output="$(./scripts/kernel-floor.sh "$profile" --json)"; then
    echo "::error::kernel-floor.sh failed to measure profile '$profile'" >&2
    exit 1
  fi
  read -r packages names native <<< "$(
    python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["packages"], d["names"], d["native"])' \
      <<< "$json_output"
  )"

  [[ "$VERBOSE" == "--verbose" ]] && \
    echo "profile=$profile packages=$packages/$max_packages names=$names/$max_names native=$native/$max_native"

  if (( packages > max_packages )); then
    echo "::error::kernel floor REGRESSED: profile '$profile' resolves $packages" \
         "packages, limit is $max_packages. A second version of an existing" \
         "crate still counts as dependency growth."
    echo "  Find the duplicate or new dependency with:"
    echo "    scripts/kernel-floor.sh $profile"
    echo "    cargo tree --duplicates --no-default-features --features $profile"
    echo "  If the growth is genuinely required, raise the limit in $LIMITS and"
    echo "  justify it in the PR body — do not raise it silently."
    status=1
  elif (( packages + SLACK < max_packages )); then
    echo "::error::kernel floor IMPROVED but was not ratcheted: profile '$profile'" \
         "resolves $packages packages, limit still $max_packages."
    echo "  Lower it to $packages in $LIMITS in this same PR, or the shed grows back"
    echo "  unnoticed."
    status=1
  fi

  if (( names > max_names )); then
    echo "::error::kernel floor REGRESSED: profile '$profile' resolves $names crate" \
         "names, limit is $max_names."
    echo "  A dependency was added to an always-on path. Find it with:"
    echo "    scripts/kernel-floor.sh $profile"
    echo "    cargo tree -i <suspect> --no-default-features --features $profile"
    echo "  If the growth is genuinely required, raise the limit in $LIMITS and"
    echo "  justify it in the PR body — do not raise it silently."
    status=1
  elif (( names + SLACK < max_names )); then
    echo "::error::kernel floor IMPROVED but was not ratcheted: profile '$profile'" \
         "resolves $names names, limit still $max_names."
    echo "  Lower it to $names in $LIMITS in this same PR, or the shed grows back"
    echo "  unnoticed."
    status=1
  fi

  if (( native > max_native )); then
    echo "::error::native-build count REGRESSED: profile '$profile' has $native" \
         "crates with a C/C++/asm build, limit is $max_native."
    echo "  Each one adds a toolchain requirement for every downstream consumer."
    status=1
  elif (( native < max_native )); then
    echo "::error::native-build count IMPROVED but was not ratcheted: profile" \
         "'$profile' has $native, limit still $max_native. Lower it in $LIMITS."
    status=1
  fi
done < "$LIMITS"

if (( status == 0 )); then
  echo "kernel floor OK — every profile within its ratchet"
fi
exit "$status"
