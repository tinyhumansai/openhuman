#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();

function usage() {
  return 'Usage: node scripts/check-domain-e2e-coverage.mjs';
}

for (const arg of process.argv.slice(2)) {
  if (arg === '--help' || arg === '-h') {
    console.log(usage());
    process.exit(0);
  }
  console.error(`check-domain-e2e-coverage: unknown argument: ${arg}`);
  console.error(usage());
  process.exit(2);
}

const rawThreshold = process.env.DOMAIN_E2E_COVERAGE_THRESHOLD ?? '90';
const THRESHOLD = Number(rawThreshold);
if (!Number.isFinite(THRESHOLD) || THRESHOLD < 0 || THRESHOLD > 100) {
  // A non-numeric value would make THRESHOLD NaN, turning every `percent <
  // THRESHOLD` comparison false and silently disabling the gate. Fail loudly.
  console.error(
    `Invalid DOMAIN_E2E_COVERAGE_THRESHOLD="${rawThreshold}". Expected a number between 0 and 100.`,
  );
  process.exit(2);
}

// Curated labels for namespaces that read better grouped, plus the namespaces
// this gate was originally written to watch. This list is now PRESENTATIONAL
// and a safety net, not the scope of the check: every namespace discovered in
// the source is measured whether or not it appears here (see `rows` below).
//
// It used to be the scope, and that was the bug. Fifty-odd namespaces —
// `flows`, `skills`, `skill_runtime`, `webhooks`, `cron`, `subagent`,
// `mcp_setup`, `workflow_run`, `voice`, `billing`, `team`, … — were never
// measured at any threshold simply because nobody added a line here, and
// nothing made that visible. A list you must remember to extend is a list that
// silently stops covering things.
//
// A namespace named here that no longer exists in the source is a hard failure:
// it means either the namespace was deleted (drop the line) or discovery has
// stopped seeing it (fix discovery). Both are worth a red lane.
const MODULES = [
  { label: 'config', namespaces: ['config'] },
  { label: 'credentials', namespaces: ['auth'] },
  { label: 'app_state', namespaces: ['app_state'] },
  { label: 'connectivity', namespaces: ['connectivity'] },
  { label: 'inference', namespaces: ['inference'] },
  { label: 'agent', namespaces: ['agent'] },
  { label: 'tools', namespaces: ['tools'] },
  { label: 'tool_registry', namespaces: ['tool_registry'] },
  { label: 'approval', namespaces: ['approval'] },
  { label: 'memory', namespaces: ['memory'] },
  { label: 'memory_tree', namespaces: ['memory_tree'] },
  { label: 'memory_sync', namespaces: ['memory_sync'] },
  { label: 'memory_sources', namespaces: ['memory_sources'] },
  { label: 'embeddings', namespaces: ['embeddings'] },
  { label: 'channels', namespaces: ['channels'] },
  { label: 'composio', namespaces: ['composio'] },
  { label: 'threads', namespaces: ['threads'] },
];

// Where `ControllerSchema` literals live.
//
// `src/openhuman` is the bulk. The second root is not optional: the `channels`
// namespace's 20 controllers are declared in the vendored TinyChannels *bus*
// crate as `ChannelControllerSchema` literals, and openhuman's
// `channels/controllers/schemas.rs` only maps them across with
// `namespace: schema.namespace` — dynamic fields no static scan can read. With
// only the first root, `channels` discovers zero controllers and (before the
// fix below) scored a vacuous 100%.
//
// `app/src/services/__tests__/rpcMethods.test.ts` already reaches into the same
// vendored crate for the same reason.
const SCHEMA_ROOTS = [
  path.join(ROOT, 'src', 'openhuman'),
  path.join(ROOT, 'vendor', 'tinychannels', 'crates', 'tinychannels-bus', 'src', 'controllers'),
];

function walk(dir, predicate, out = []) {
  if (!fs.existsSync(dir)) return out;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      walk(full, predicate, out);
    } else if (predicate(full)) {
      out.push(full);
    }
  }
  return out;
}

function read(file) {
  return fs.readFileSync(file, 'utf8');
}

/**
 * Every `"openhuman.x_y"` string literal appearing in a `tests/**\/*_e2e.rs`.
 *
 * KNOWN LIMITATION, left in place deliberately. This is a text match, so a
 * method named anywhere in such a file counts as covered — including in a
 * schema-catalog assertion list that never calls it.
 *
 * Two tightenings were measured before deciding to leave it:
 *
 *  - Ignoring comments: **zero** methods are credited by a comment alone today
 *    (390 found with comments, 390 without), so stripping them is a no-op that
 *    would only add a regex able to mangle a string literal containing `//`.
 *  - Ignoring bare list entries: not separable by line shape. rustfmt puts a
 *    long call's method argument on its own line, so an invocation and a list
 *    element look identical (`"openhuman.flows_create",` is a list entry by
 *    shape and a real `post_json_rpc` argument at `json_rpc_e2e.rs:11049`).
 *    Telling them apart needs an AST, which is a different tool than this.
 *
 * So: a method credited here is *named* by an e2e target, not provably invoked
 * by one. Read the percentages with that in mind.
 */
function collectInvokedMethods() {
  const methods = new Set();
  const testsDir = path.join(ROOT, 'tests');
  const files = walk(testsDir, (file) => file.endsWith('_e2e.rs'));

  for (const file of files) {
    const text = read(file);
    for (const match of text.matchAll(/"((?:openhuman)\.[A-Za-z0-9_]+)"/g)) {
      methods.add(match[1]);
    }
  }

  return methods;
}

/**
 * Every controller declared anywhere under `SCHEMA_ROOTS`, keyed by namespace.
 *
 * This reads EVERY `.rs` file under those roots. It used to read only files
 * whose path matched `/(^|\/)schemas?(\.rs|\/)/`, which stopped working on
 * 2026-08-30: the `include!` split (#5856/#5857) moved `ControllerSchema`
 * literals out of `schemas.rs` into `*_part_NN.rs` siblings that the pattern
 * does not match, and out of `flows/schemas.rs` into `flows_schema_part_*.rs`
 * entirely. Thirteen files and 180 controllers went invisible in one commit,
 * with no signal — the gate simply reported a smaller world.
 *
 * The path filter bought nothing a content match does not: a file with no
 * `ControllerSchema` literal contributes nothing either way. Dropping it means
 * the next refactor that moves a declaration cannot repeat this.
 */
function collectSchemaMethods() {
  const methodsByNamespace = new Map();

  for (const root of SCHEMA_ROOTS) {
    for (const file of walk(root, (f) => f.endsWith('.rs'))) {
      const text = read(file);
      const constNamespace = text.match(/const\s+NAMESPACE:\s*&str\s*=\s*"([a-z_]+)"/)?.[1];
      // `ChannelControllerSchema` is the vendored bus crate's equivalent shape.
      for (const match of text.matchAll(/(?:Channel)?ControllerSchema\s*\{([\s\S]*?)\n\s*\}/g)) {
        const block = match[1];
        const namespaceToken = block.match(/namespace:\s*(?:NAMESPACE|"([a-z_]+)")/);
        const functionName = block.match(/function:\s*"([A-Za-z0-9_]+)"/)?.[1];
        const namespace = namespaceToken?.[1] ?? (namespaceToken ? constNamespace : undefined);
        if (!namespace || !functionName || functionName === 'unknown') continue;
        if (!methodsByNamespace.has(namespace)) methodsByNamespace.set(namespace, new Set());
        methodsByNamespace.get(namespace).add(`openhuman.${namespace}_${functionName}`);
      }
    }
  }

  return methodsByNamespace;
}

const invoked = collectInvokedMethods();
const schemas = collectSchemaMethods();

const labelForNamespace = new Map();
for (const module of MODULES) {
  for (const namespace of module.namespaces) labelForNamespace.set(namespace, module.label);
}

// A MODULES entry naming a namespace discovery cannot find. Either the
// namespace is gone (delete the line) or discovery broke (fix it). Reporting
// this as 0/0 = 100% is exactly the bug this gate had.
const declaredButMissing = [...labelForNamespace.keys()]
  .filter((namespace) => !schemas.has(namespace))
  .sort();

// One row per namespace that actually exists, grouped where MODULES says so.
const rows = new Map();
for (const [namespace, methods] of schemas) {
  const label = labelForNamespace.get(namespace) ?? namespace;
  if (!rows.has(label)) rows.set(label, { label, namespaces: [], expected: new Set() });
  const row = rows.get(label);
  row.namespaces.push(namespace);
  for (const method of methods) row.expected.add(method);
}

let failed = false;
const below = [];

console.log(`Domain Rust E2E controller coverage threshold: ${THRESHOLD}%`);
console.log('');
console.log('| Module | Namespace(s) | Covered | Percent | Missing |');
console.log('| --- | --- | ---: | ---: | --- |');

for (const row of [...rows.values()].sort((a, b) => a.label.localeCompare(b.label))) {
  const covered = new Set([...row.expected].filter((method) => invoked.has(method)));
  const missing = [...row.expected].filter((method) => !covered.has(method)).sort();

  // `expected.size === 0` used to score 100%. It cannot happen here — a row
  // only exists because discovery found at least one controller for it — and
  // the case it used to hide is now `declaredButMissing` above.
  const percent = (covered.size / row.expected.size) * 100;
  const missingText = missing.length === 0 ? '-' : missing.join('<br>');

  console.log(
    `| ${row.label} | ${row.namespaces.sort().join(', ')} | ${covered.size}/${row.expected.size} | ${percent.toFixed(1)}% | ${missingText} |`,
  );

  if (percent < THRESHOLD) {
    failed = true;
    below.push(`${row.label} (${covered.size}/${row.expected.size}, ${percent.toFixed(1)}%)`);
  }
}

const totalExpected = [...rows.values()].reduce((sum, row) => sum + row.expected.size, 0);
const totalCovered = [...rows.values()].reduce(
  (sum, row) => sum + [...row.expected].filter((method) => invoked.has(method)).length,
  0,
);
console.log('');
console.log(
  `Discovered ${totalExpected} controllers across ${rows.size} namespaces; ${totalCovered} invoked by a tests/**/*_e2e.rs target.`,
);

if (declaredButMissing.length > 0) {
  failed = true;
  console.error(
    `\nMODULES names ${declaredButMissing.length} namespace(s) with no discovered controllers: ${declaredButMissing.join(', ')}.` +
      '\nEither the namespace was removed (drop it from MODULES) or schema discovery has stopped seeing it (fix SCHEMA_ROOTS / the match).' +
      '\nThis is NOT a coverage result — nothing was measured.',
  );
}

if (failed) {
  if (below.length > 0) {
    console.error(
      `\nDomain Rust E2E controller coverage is below ${THRESHOLD}% for ${below.length} module(s):\n  ${below.join('\n  ')}`,
    );
  }
  process.exit(1);
}

console.log(`\nAll ${rows.size} namespaces meet the ${THRESHOLD}% Rust E2E controller coverage threshold.`);
