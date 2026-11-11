#!/usr/bin/env bash
# Run the isolated Playwright logical units assigned to one hosted lane.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

lane=""
lanes="4"
selection="all"
list=0
while (($#)); do
  case "$1" in
    --lane=*) lane="${1#*=}" ;;
    --lanes=*) lanes="${1#*=}" ;;
    --selection=*) selection="${1#*=}" ;;
    --list) list=1 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

[[ "$lane" =~ ^[1-9][0-9]*$ && "$lanes" =~ ^[1-9][0-9]*$ ]] || { echo "--lane and --lanes must be positive integers" >&2; exit 2; }
(( lane <= lanes )) || { echo "lane $lane exceeds lane count $lanes" >&2; exit 2; }

selected_paths="$(node app/scripts/e2e-web-selection.mjs "--selection=$selection")" || exit $?
selected_files=()
while IFS= read -r file; do
  [[ -n "$file" ]] && selected_files+=("$file")
done <<< "$selected_paths"
(( ${#selected_files[@]} > 0 )) || { echo "$selection selection is empty" >&2; exit 2; }

contains_file() {
  local wanted="$1" file
  for file in "${selected_files[@]}"; do
    [[ "$file" == "$wanted" ]] && return 0
  done
  return 1
}

special_files=(
  "test/playwright/specs/connections-tab-aliases.spec.ts"
  "test/playwright/specs/connections-tab-deeplinks.spec.ts"
)
special_titles=(
  "a burst of typing issues ONE catalog search"
  "a pause between words issues a second search"
  "a query with no matches leaves no catalog rows"
  "the typed text is preserved in the box while results load"
  "offers Install, and the button is enabled before it is pressed"
  "disables the button and shows Installing while the RPC is in flight"
  "a failed install re-enables the button rather than stranding it"
)

if (( list )); then
  first=1
  printf '['
  assigned=0
  if [[ "$selection" == diagnostic ]]; then
    for i in "${!selected_files[@]}"; do
      if (( i % lanes + 1 == lane )); then
        (( first )) || printf ','
        printf '{"kind":"diagnostic-file","index":%d,"file":"%s"}' "$i" "${selected_files[$i]}"
        first=0
        assigned=$((assigned + 1))
      fi
    done
  else
  for i in "${!special_files[@]}"; do
    if contains_file "${special_files[$i]}" && (( i % lanes + 1 == lane )); then
      (( first )) || printf ','
      printf '{"kind":"special-file","index":%d,"file":"%s"}' "$i" "${special_files[$i]}"
      first=0
      assigned=$((assigned + 1))
    fi
  done
  for i in "${!special_titles[@]}"; do
    unit=$((i + ${#special_files[@]}))
    if contains_file "test/playwright/specs/skills-search-install.spec.ts" && (( unit % lanes + 1 == lane )); then
      escaped="${special_titles[$i]//\\/\\\\}"
      escaped="${escaped//\"/\\\"}"
      (( first )) || printf ','
      printf '{"kind":"special-title","index":%d,"file":"test/playwright/specs/skills-search-install.spec.ts","title":"%s"}' "$unit" "$escaped"
      first=0
      assigned=$((assigned + 1))
    fi
  done
  for ((shard=1; shard<=64; shard++)); do
    if (( (shard - 1) % lanes + 1 == lane )); then
      (( first )) || printf ','
      printf '{"kind":"shard","index":%d,"shard":"%d/64"}' "$shard" "$shard"
      first=0
      assigned=$((assigned + 1))
    fi
  done
  fi
  printf ']\n'
  (( assigned > 0 )) || { echo "$selection selection assigns no units to lane $lane" >&2; exit 2; }
  exit 0
fi

workspace_root="${OPENHUMAN_WORKSPACE:-${RUNNER_TEMP:-$REPO_ROOT/target}/openhuman-playwright}"
export OPENHUMAN_E2E_ALLOW_EPHEMERAL_KEY="${OPENHUMAN_E2E_ALLOW_EPHEMERAL_KEY:-1}"
if [[ -n "${E2E_WEB_SESSION_RUNNER:-}" ]]; then
  read -r -a runner_cmd <<< "$E2E_WEB_SESSION_RUNNER"
else
  runner_cmd=(bash app/scripts/e2e-web-session.sh)
fi

escape_regex() {
  printf '%s' "$1" | sed 's/[][(){}.*+?^$|\\]/\\&/g'
}

mkdir -p "$workspace_root/selection-$selection/lane-$lane/receipts"
receipt_dir="$workspace_root/selection-$selection/lane-$lane/receipts"
overall=0
run_unit() {
  local unit="$1"; shift
  local workspace="$workspace_root/selection-$selection/lane-$lane/$unit"
  local receipt="$receipt_dir/$unit.exit"
  mkdir -p "$workspace"
  workspace="$(cd "$workspace" && pwd)"
  local output="$workspace/test-results"
  export OPENHUMAN_WORKSPACE="$workspace"
  local status=0
  bash scripts/ci-cancel-aware.sh "${runner_cmd[@]}" "$@" --output="$output" || status=$?
  printf '%s\n' "$status" > "$receipt"
  echo "unit=$unit exit=$status receipt=$receipt"
  (( status == 0 )) || overall=1
}

if [[ "$selection" == diagnostic ]]; then
  assigned=0
  for i in "${!selected_files[@]}"; do
    if (( i % lanes + 1 == lane )); then
      run_unit "diagnostic-$i" "${selected_files[$i]}"
      assigned=$((assigned + 1))
    fi
  done
  (( assigned > 0 )) || { echo "$selection selection assigns no units to lane $lane" >&2; exit 2; }
else
for i in "${!special_files[@]}"; do
  if contains_file "${special_files[$i]}" && (( i % lanes + 1 == lane )); then
    run_unit "special-$i" "${special_files[$i]}"
  fi
done
for i in "${!special_titles[@]}"; do
  unit=$((i + ${#special_files[@]}))
  if contains_file "test/playwright/specs/skills-search-install.spec.ts" && (( unit % lanes + 1 == lane )); then
    run_unit "special-$unit" test/playwright/specs/skills-search-install.spec.ts -g "$(escape_regex "${special_titles[$i]}")"
  fi
done
shard_files=()
if [[ "$selection" == required ]]; then
  for file in "${selected_files[@]}"; do
    if [[ "$file" != "test/playwright/specs/connections-tab-aliases.spec.ts" && "$file" != "test/playwright/specs/connections-tab-deeplinks.spec.ts" ]]; then
      shard_files+=("$file")
    fi
  done
fi
if [[ "$selection" != all ]] && (( ${#shard_files[@]} == 0 )); then
  echo "$selection selection has no ordinary shard files" >&2
  exit 2
fi
for ((shard=1; shard<=64; shard++)); do
  if (( (shard - 1) % lanes + 1 == lane )); then
    if [[ "$selection" == all ]]; then
      run_unit "shard-$shard" --shard="$shard/64" --grep-invert='Connections (aliases|deep links)|Skills explorer'
    else
      run_unit "shard-$shard" "${shard_files[@]}" --shard="$shard/64" --grep-invert='Connections (aliases|deep links)|Skills explorer'
    fi
  fi
done
fi
exit "$overall"
