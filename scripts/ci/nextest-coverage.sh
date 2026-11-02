#!/usr/bin/env bash
# One durable JUnit report and count inventory per instrumented configuration.
# Usage: bash scripts/ci/nextest-coverage.sh <suite> <cargo command...>
# Run from the repository root. The command must contain a `nextest` argument.
# NEXTEST_REPORT_DIR selects durable <suite>.toml/.xml/.json outputs. Optional
# NEXTEST_EXPECTED_TARGETS and NEXTEST_EXPECTED_INVENTORY name the target-list
# and nextest-list JSON inputs accepted by nextest-report.mjs.
set -euo pipefail
suite="$1"
shift
case "$suite" in '' | *[!a-zA-Z0-9._-]*) echo 'invalid nextest suite name' >&2; exit 64 ;; esac
report_dir="${NEXTEST_REPORT_DIR:-${PWD}/ci-out/nextest}"
mkdir -p "$report_dir"
report_dir="$(cd "$report_dir" && pwd)"
config="${report_dir}/${suite}.toml"
report="${report_dir}/${suite}.xml"
# Absolute paths keep reports out of the compiler target tree and survive its
# cleanup. A fresh config retains the repository's timeouts/resource groups.
node - "$config" "$report" "$@" <<'JS'
const fs = require('node:fs');
const { spawnSync } = require('node:child_process');
const [config, report, ...command] = process.argv.slice(2);
const manifest = command.indexOf('--manifest-path');
const metadataArgs = ['metadata', '--no-deps', '--format-version', '1'];
if (manifest >= 0) metadataArgs.push('--manifest-path', command[manifest + 1]);
const metadataResult = spawnSync('bash', ['scripts/ci-cancel-aware.sh', 'cargo', ...metadataArgs], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] });
if (metadataResult.status !== 0) process.exit(metadataResult.status ?? 1);
const metadata = JSON.parse(metadataResult.stdout);
const targets = new Set(metadata.packages.flatMap(pkg => pkg.targets.map(target => target.name)));
// Nextest validates binary predicates against the selected workspace, even
// regex predicates. A standalone Tauri workspace has none of the root CLI's
// resource-sensitive binaries, so omit only those inapplicable overrides in
// every profile (including the default profile used by the Tauri suite).
const source = fs.readFileSync('.config/nextest.toml', 'utf8');
const configText = source.replace(/\n\[\[profile\.(?:ci|default)\.overrides\]\]\n[\s\S]*?(?=\n\[|$)/g, block => {
  const binary = block.match(/binary\(\/([^/]+)\/\)/);
  const exact = block.match(/binary\(=([a-zA-Z0-9_-]+)\)/);
  const present = binary
    ? [...targets].some(name => new RegExp(binary[1]).test(name))
    : exact && targets.has(exact[1]);
  return (binary || exact) && !present ? '' : block;
});
fs.writeFileSync(config, configText + '\n[profile.ci.junit]\npath = ' + JSON.stringify(report) + '\n');
JS
rm -f "$report" "${report_dir}/${suite}.json"
args=()
for arg in "$@"; do
  args+=("$arg")
  if [ "$arg" = nextest ]; then args+=(--config-file "$config"); fi
done
started=$SECONDS
rc=0
bash scripts/ci-cancel-aware.sh "${args[@]}" || rc=$?
report_rc=0
NEXTEST_ELAPSED_SECONDS="$((SECONDS - started))" node scripts/ci/nextest-report.mjs "$report" "${report_dir}/${suite}.json" ${NEXTEST_EXPECTED_TARGETS:+"${NEXTEST_EXPECTED_TARGETS}"} ${NEXTEST_EXPECTED_INVENTORY:+"${NEXTEST_EXPECTED_INVENTORY}"} || report_rc=$?
# A report/inventory failure must not hide the test runner's actual failure or
# cancellation. Successful executions must also produce nonempty reports.
[ "$rc" != 0 ] || rc="$report_rc"
exit "$rc"
