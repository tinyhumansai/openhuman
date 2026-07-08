#!/usr/bin/env node
// Test-inventory guard: keeps the test suite honestly wired into CI.
//
// A test that no job runs is worse than no test — it reads as coverage while
// verifying nothing. This script enforces two ratchets that fail CI:
//
//   (a) ORPHAN CHECK — every discovered script-level test file
//       (`scripts/**/*.test.mjs` and the PowerShell install test) is invoked
//       by >=1 package.json script (directly or via a `node --test <glob>`)
//       OR referenced by a workflow. Framework-globbed suites (Vitest, WDIO,
//       Playwright, cargo test) are discovered by their runners' own config
//       globs, not enumerated here, so they are out of scope for the orphan
//       check — the orphans the audit found all live under `scripts/`.
//
//   (b) CONTROLLER-DOMAIN CHECK — every controller domain registered in
//       `src/core/all.rs` (via `crate::openhuman::<domain>::all_*_controllers`)
//       is referenced by >=1 file under `tests/`. Catches RPC domains that
//       ship with zero integration/E2E coverage (recall_calendar, tinyplace,
//       devices, …).
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
  'audio_toolkit',
  'council_registry',
  'desktop_companion',
  'devices',
  'harness_init',
  'http_host',
  'mcp_audit',
  'memory_diff',
  'memory_goals',
  'model_council',
  'people',
  'plan_review',
  'provider_surfaces',
  'recall_calendar',
  'redirect_links',
  'referral',
  'session_import',
  'skill_runtime',
  'task_sources',
  'text_input',
  'thread_goals',
  'tinyplace',
  'webview_notifications',
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
  const allRs = read(path.join(ROOT, 'src', 'core', 'all.rs'));
  const domains = new Set();
  // crate::openhuman::<domain>[::<sub>...]::all_<name>_(registered|internal)_controllers
  const re =
    /crate::openhuman::([a-z0-9_]+)(?:::[a-z0-9_]+)*::all_[a-z0-9_]+_(?:registered|internal)_controllers/g;
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
// Run
// ─────────────────────────────────────────────────────────────────────────────

const scriptTests = discoverScriptTests();
const orphans = computeOrphans(scriptTests);

const domains = discoverControllerDomains();
const unreferencedDomains = computeUnreferencedDomains(domains);
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
}

let failed = false;

if (orphans.length > 0) {
  failed = true;
  console.error('\n✖ Orphaned test files (invoked by no package.json script or workflow):');
  for (const file of orphans) console.error(`  - ${file}`);
  console.error('  Wire each into `test:scripts` (or a dedicated script), or allowlist with cause.');
}

if (unreferencedDomains.length > 0) {
  failed = true;
  console.error('\n✖ Controller domains registered in src/core/all.rs with no reference in tests/:');
  for (const domain of unreferencedDomains) console.error(`  - ${domain}`);
  console.error('  Add >=1 RPC round-trip under tests/, or allowlist in DOMAIN_ALLOWLIST with cause.');
}

if (failed) {
  process.exit(1);
}

if (!JSON_OUT) console.log('\n✔ All script tests are wired in and every controller domain is referenced in tests/.');
