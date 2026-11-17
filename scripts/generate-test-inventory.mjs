#!/usr/bin/env node
// Test-inventory guard: keeps the test suite honestly wired into CI.
//
// A test that no job runs is worse than no test — it reads as coverage while
// verifying nothing. This script enforces two ratchets that fail CI:
//
//   (a) ORPHAN CHECK — every discovered script-level test file
//       (`scripts/**/*.test.mjs` and the PowerShell install test) is invoked
//       by >=1 package.json script (directly or via a `node --test <glob>`)
//       OR referenced by a workflow. Vitest, Playwright and cargo test really
//       are discovered by their runners' own config globs, so they stay out of
//       scope here.
//
//       WDIO IS NOT, and used to be exempted on that false premise. Its config
//       glob (`wdio.conf.ts`: test/e2e/specs/**/*.spec.ts) is overridden the
//       moment a caller passes spec paths, and the only path CI takes does
//       exactly that: e2e-run-all-flows.sh collects a HAND-MAINTAINED list and
//       e2e-run-session.sh turns it into `--spec` flags. A spec absent from
//       that list is therefore run by nothing, while the config glob makes it
//       look covered. Fourteen specs had drifted out this way before check (c)
//       below existed. See check (c).
//
//   (c) WDIO LANE CHECK — every `app/test/e2e/specs/*.spec.ts` is named by an
//       active `run "..."` line in `app/scripts/e2e-run-all-flows.sh`, the only
//       orchestrator CI uses. Catches a spec that exists, typechecks and is
//       never executed.
//
//   (b) CONTROLLER-DOMAIN CHECK — every controller domain registered in
//       `crates/openhuman-core/src/core/all.rs` (via `crate::<domain>::all_*_controllers`)
//       is referenced by >=1 file under `tests/`. Catches RPC domains that
//       ship with zero integration/E2E coverage (devices, …).
//
// Known-current offenders are seeded into the allowlists below so the check
// lands green; the intent is to burn those lists down over time. Any NEW
// offender (a fresh orphan test, or a new controller domain with no tests/
// reference) fails CI until it is wired up or explicitly allowlisted.
//
// Usage:
//   node scripts/generate-test-inventory.mjs            # report + enforce
//   node scripts/generate-test-inventory.mjs --check    # same (explicit)
//   node scripts/generate-test-inventory.mjs --json     # machine-readable dump

import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const argv = new Set(process.argv.slice(2));
const JSON_OUT = argv.has('--json');

// ─────────────────────────────────────────────────────────────────────────────
// Allowlists — burn these down. Adding an entry is a deliberate, reviewable act.
// ─────────────────────────────────────────────────────────────────────────────

// Script-level test files permitted to lack any package.json/workflow invocation.
// Should stay empty: wire the test into `test:scripts` (or a dedicated script)
// instead of allowlisting it.
const ORPHAN_ALLOWLIST = new Set([]);

// WDIO specs permitted to be absent from `e2e-run-all-flows.sh`. An entry is a
// deliberate, reviewable disable WITH a cause — not a parking space for a spec
// someone forgot to wire up. Delete the entry when the spec goes back in.
const WDIO_LANE_ALLOWLIST = new Map([
  [
    'slack-flow.spec.ts',
    'Crashes the CEF session mid-spec on Linux (#1850-style state issue); its ' +
      '`run` line is commented out in e2e-run-all-flows.sh with the same cause.',
  ],
]);

// Controller domains permitted to lack any reference under tests/. Each entry
// is a Rust integration-coverage gap tracked in plan.md §4/§A.3 — remove the
// entry when the domain gains a tests/ reference.
const DOMAIN_ALLOWLIST = new Set([
  // Seeded 2026-07 from the initial run — plan.md §4/§A.3 Rust-E2E gaps.
  // Burn down by adding an RPC round-trip under tests/ for each, then delete
  // the corresponding line here.
  'agent_experience',
  'agent_meetings',
  'announcements',
  'devices',
  'harness_init',
  'http_host',
  'mcp_audit',
  'memory_diff',
  'memory_goals',
  'people',
  'plan_review',
  'provider_surfaces',
  'referral',
  'session_import',
  'skill_runtime',
  'task_sources',
  'text_input',
  'thread_goals',
  // NOT a coverage gap. ea3fbfb61a moved these controllers from `agent` to
  // `agent_runs` without changing their wire names, and
  // tests/in_process/agent_approval_memory_coverage_e2e.rs still round-trips
  // agent_run_events / agent_run_status / agent_runs_active. None of those
  // method names match `\bagent_runs\b`, so the guard cannot see them.
  // Remove this entry once a test under tests/ names the domain, or once this
  // script matches domains by RPC method name instead of module path.
  'agent_runs',
]);

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

function walk(dir, predicate, out = []) {
  if (!fs.existsSync(dir)) return out;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === 'node_modules' || entry.name === '.git') continue;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) walk(full, predicate, out);
    else if (predicate(full)) out.push(full);
  }
  return out;
}

function read(file) {
  return fs.readFileSync(file, 'utf8');
}

function rel(file) {
  return path.relative(ROOT, file).split(path.sep).join('/');
}

// Convert a shell-style glob (single-segment `*`, recursive `**`) into a RegExp
// anchored against a repo-relative POSIX path.
function globToRegExp(glob) {
  let re = '^';
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i];
    if (c === '*') {
      if (glob[i + 1] === '*') {
        re += '.*';
        i++;
        if (glob[i + 1] === '/') i++; // consume the trailing slash of `**/`
      } else {
        re += '[^/]*';
      }
    } else if ('.+?^${}()|[]\\'.includes(c)) {
      re += '\\' + c;
    } else {
      re += c;
    }
  }
  return new RegExp(re + '$');
}

// ─────────────────────────────────────────────────────────────────────────────
// (a) Orphan check
// ─────────────────────────────────────────────────────────────────────────────

function discoverScriptTests() {
  const scriptsDir = path.join(ROOT, 'scripts');
  return walk(scriptsDir, (f) => f.endsWith('.test.mjs') || f.endsWith('.Tests.ps1'))
    .map(rel)
    .sort();
}

function loadInvocationSources() {
  // Every package.json script command string.
  const pkg = JSON.parse(read(path.join(ROOT, 'package.json')));
  const scriptCommands = Object.values(pkg.scripts ?? {});

  // Every workflow YAML, raw.
  const workflowsDir = path.join(ROOT, '.github', 'workflows');
  const workflowText = walk(workflowsDir, (f) => f.endsWith('.yml') || f.endsWith('.yaml'))
    .map(read)
    .join('\n');

  return { scriptCommands, combinedText: scriptCommands.join('\n') + '\n' + workflowText };
}

// Extract glob/path args from each `node --test ...` occurrence in a command.
function nodeTestGlobs(command) {
  const globs = [];
  for (const m of command.matchAll(/node\s+--test\s+([^\n&|;]+)/g)) {
    for (const tok of m[1].trim().split(/\s+/)) {
      if (tok.startsWith('-')) continue; // skip flags like --test-reporter
      globs.push(tok);
    }
  }
  return globs;
}

function computeOrphans(scriptTests) {
  const { scriptCommands, combinedText } = loadInvocationSources();

  // Files matched by a `node --test <glob>` in any package.json script.
  const globMatchers = scriptCommands.flatMap(nodeTestGlobs).map(globToRegExp);

  const orphans = [];
  for (const file of scriptTests) {
    const matchedByGlob = globMatchers.some((re) => re.test(file));
    const referencedLiterally = combinedText.includes(file);
    const covered = matchedByGlob || referencedLiterally;
    if (!covered && !ORPHAN_ALLOWLIST.has(file)) orphans.push(file);
  }
  return orphans;
}

// ─────────────────────────────────────────────────────────────────────────────
// (b) Controller-domain check
// ─────────────────────────────────────────────────────────────────────────────

function discoverControllerDomains() {
  const allRs = read(path.join(ROOT, 'crates', 'openhuman-core', 'src', 'core', 'all.rs'));
  const domains = new Set();
  // crate::<domain>[::<sub>...]::all_<name>_(registered|internal)_controllers
  const re =
    /crate::([a-z0-9_]+)(?:::[a-z0-9_]+)*::all_[a-z0-9_]+_(?:registered|internal)_controllers/g;
  for (const m of allRs.matchAll(re)) domains.add(m[1]);
  return [...domains].sort();
}

function domainsReferencedInTests() {
  const testsDir = path.join(ROOT, 'tests');
  const combined = walk(testsDir, (f) => f.endsWith('.rs'))
    .map(read)
    .join('\n');
  return combined;
}

function computeUnreferencedDomains(domains) {
  const testsText = domainsReferencedInTests();
  const missing = [];
  for (const domain of domains) {
    const referenced = new RegExp(`\\b${domain}\\b`).test(testsText);
    if (!referenced && !DOMAIN_ALLOWLIST.has(domain)) missing.push(domain);
  }
  return missing;
}

// ─────────────────────────────────────────────────────────────────────────────
// (c) WDIO lane check
// ─────────────────────────────────────────────────────────────────────────────

const WDIO_SPEC_DIR = path.join(ROOT, 'app', 'test', 'e2e', 'specs');
const WDIO_ORCHESTRATOR = path.join(ROOT, 'app', 'scripts', 'e2e-run-all-flows.sh');

function discoverWdioSpecs() {
  if (!fs.existsSync(WDIO_SPEC_DIR)) return [];
  return fs
    .readdirSync(WDIO_SPEC_DIR)
    .filter((f) => f.endsWith('.spec.ts'))
    .sort();
}

/// Spec basenames named by an ACTIVE `run "..."` line.
///
/// Anchored at line start so a commented-out `# run "..."` does not count — a
/// disabled spec is exactly the case this check exists to surface, and matching
/// the comment would make the guard agree with the bug.
function specsNamedByOrchestrator() {
  if (!fs.existsSync(WDIO_ORCHESTRATOR)) return new Set();
  const named = new Set();
  const re = /^[ \t]*run[ \t]+"test\/e2e\/specs\/([^"]+)"/gm;
  for (const m of read(WDIO_ORCHESTRATOR).matchAll(re)) named.add(m[1]);
  return named;
}

function computeUnrunWdioSpecs() {
  const specs = discoverWdioSpecs();
  const named = specsNamedByOrchestrator();
  // Guard the guard: if the orchestrator parse yields nothing while specs do
  // exist, the regex has drifted from the script's format and every spec would
  // be reported as unrun. That is a tooling failure, not a coverage finding,
  // and must not be reported as one.
  if (specs.length > 0 && named.size === 0) {
    throw new Error(
      `WDIO lane check parsed 0 \`run\` lines from ${path.relative(ROOT, WDIO_ORCHESTRATOR)} ` +
        `while ${specs.length} spec files exist. The matcher has drifted from the script's ` +
        `format — fix the regex rather than treating this as missing coverage.`,
    );
  }
  return {
    specs,
    named,
    unrun: specs.filter((f) => !named.has(f) && !WDIO_LANE_ALLOWLIST.has(f)),
  };
}

// ─────────────────────────────────────────────────────────────────────────────
// Run
// ─────────────────────────────────────────────────────────────────────────────

const scriptTests = discoverScriptTests();
const orphans = computeOrphans(scriptTests);

const domains = discoverControllerDomains();
const unreferencedDomains = computeUnreferencedDomains(domains);
const wdio = computeUnrunWdioSpecs();
const referencedDomainCount = domains.length - unreferencedDomains.length - DOMAIN_ALLOWLIST.size;

if (JSON_OUT) {
  console.log(
    JSON.stringify(
      {
        scriptTests,
        orphans,
        orphanAllowlist: [...ORPHAN_ALLOWLIST],
        domains,
        unreferencedDomains,
        domainAllowlist: [...DOMAIN_ALLOWLIST],
        wdioSpecs: wdio.specs,
        wdioSpecsNamedByOrchestrator: [...wdio.named].sort(),
        wdioSpecsUnrun: wdio.unrun,
        wdioLaneAllowlist: [...WDIO_LANE_ALLOWLIST.keys()],
      },
      null,
      2,
    ),
  );
} else {
  console.log('Test inventory guard');
  console.log('====================');
  console.log(`Script-level test files discovered: ${scriptTests.length}`);
  console.log(`  orphaned (no invocation):         ${orphans.length}`);
  console.log(`  allowlisted orphans:              ${ORPHAN_ALLOWLIST.size}`);
  console.log(`Controller domains in all.rs:       ${domains.length}`);
  console.log(`  referenced in tests/:             ${referencedDomainCount}`);
  console.log(`  allowlisted (known gaps):         ${DOMAIN_ALLOWLIST.size}`);
  console.log(`  newly unreferenced:               ${unreferencedDomains.length}`);
  console.log(`WDIO specs on disk:                 ${wdio.specs.length}`);
  console.log(`  named by e2e-run-all-flows.sh:    ${wdio.named.size}`);
  console.log(`  allowlisted (deliberate):         ${WDIO_LANE_ALLOWLIST.size}`);
  console.log(`  run by no lane:                   ${wdio.unrun.length}`);
}

let failed = false;

if (orphans.length > 0) {
  failed = true;
  console.error('\n✖ Orphaned test files (invoked by no package.json script or workflow):');
  for (const file of orphans) console.error(`  - ${file}`);
  console.error('  Wire each into `test:scripts` (or a dedicated script), or allowlist with cause.');
}

if (wdio.unrun.length > 0) {
  failed = true;
  console.error(
    '\n\u2716 WDIO specs that exist but are run by no lane (absent from app/scripts/e2e-run-all-flows.sh):',
  );
  for (const file of wdio.unrun) console.error(`  - app/test/e2e/specs/${file}`);
  console.error(
    '  Add a `run "test/e2e/specs/<file>" "<label>" "<suite>"` line to the matching suite,',
  );
  console.error('  or add it to WDIO_LANE_ALLOWLIST with the reason it is disabled.');
}

if (unreferencedDomains.length > 0) {
  failed = true;
  console.error('\n✖ Controller domains registered in crates/openhuman-core/src/core/all.rs with no reference in tests/:');
  for (const domain of unreferencedDomains) console.error(`  - ${domain}`);
  console.error('  Add >=1 RPC round-trip under tests/, or allowlist in DOMAIN_ALLOWLIST with cause.');
}

if (failed) {
  process.exit(1);
}

if (!JSON_OUT)
  console.log(
    '\n\u2714 All script tests are wired in, every controller domain is referenced in tests/, and every WDIO spec is in a lane.',
  );
