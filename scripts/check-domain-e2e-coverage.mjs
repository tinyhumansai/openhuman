#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';

import {
  parseCoreFeatureGraph,
  parseProductFeatures,
  resolveEnabledFeatures,
} from './lib/feature-forwarding.mjs';

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

// Namespaces whose controllers are COMPILED OUT of the configuration this gate
// measures, with the gate that removes them and the reason beside each — the
// shape `INTENTIONALLY_NOT_FORWARDED` uses in scripts/lib/feature-forwarding.mjs.
//
// A method that cannot be dispatched cannot be reached by a `tests/**/*_e2e.rs`
// target, so listing it as an uncovered obligation asks for something
// impossible. The only two ways to satisfy such a row are a bespoke feature
// string (which invalidates the shared target dir for every parallel worker) or
// naming the method in a string literal that never calls it — the exact gaming
// `collectInvokedMethods` warns about above. Worse, the honest answer (0%,
// unreachable) and the dishonest one (0%, nobody bothered) look identical in
// the report.
//
// This has to live HERE and not in Rust. The schemas are already `#[cfg]`-
// correct — `src/openhuman/mod.rs` gates the whole `test_support` module — but
// discovery reads source text off disk and would find these literals even if
// every line were `#[cfg(never)]`. There is no Rust-side edit that changes what
// a text scan sees.
//
// Excluding is the dangerous direction: a wrong entry hides real work. So each
// one is checked against the feature set the measured build actually enables
// (`checkExclusions` below), not merely against this comment. #6069.
const UNREACHABLE_NAMESPACES = {
  test: {
    feature: 'e2e-test-support',
    reason:
      '`openhuman.test_reset` wipes sidecar state in place, and src/core/all.rs registers it behind ' +
      '`#[cfg(feature = "e2e-test-support")]` precisely so a shipped binary never carries the destructive RPC. ' +
      'Only app/scripts/e2e-build.sh turns that gate on; under the feature string scripts/test-rust-e2e.sh ' +
      'builds every e2e target with, dispatching it answers `unknown method`.',
  },
  test_support: {
    feature: 'e2e-test-support',
    reason:
      'Same gate as `test`: src/openhuman/mod.rs declares the whole `test_support` module behind ' +
      '`#[cfg(feature = "e2e-test-support")]`, so these workspace- and chat-introspection helpers exist only in ' +
      'the E2E build produced by app/scripts/e2e-build.sh.',
  },
};

// The two files scripts/test-rust-e2e.sh derives its `--features` string from.
//
// It runs every suite with `--features "$(scripts/ci/product-features.sh)"` and
// does NOT pass `--no-default-features`, so the measured configuration is
// `default` UNION the product set — not the product set alone. The distinction
// decides real cases: `medulla` is absent from product-features.txt but present
// in `[features] default`, so its nine controllers ARE dispatchable in an e2e
// build and are genuine obligations, not exclusions.
const CORE_MANIFEST = path.join(ROOT, 'Cargo.toml');
const PRODUCT_FEATURES_FILE = path.join(ROOT, 'scripts', 'ci', 'product-features.txt');

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
  // Which file declared each namespace, so an exclusion can be checked against
  // the `#[cfg]` chain that actually reaches it — see `moduleGateProves`.
  const filesByNamespace = new Map();

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
        if (!filesByNamespace.has(namespace)) filesByNamespace.set(namespace, new Set());
        filesByNamespace.get(namespace).add(file);
      }
    }
  }

  return { methodsByNamespace, filesByNamespace };
}

/**
 * Parse the inside of a `cfg(...)` predicate into `all` / `any` / `not` / atom.
 *
 * Deliberately a parser and not a regex. `#[cfg(not(feature = "x"))]` and
 * `#[cfg(any(feature = "x", unix))]` both CONTAIN the feature name while
 * neither requires it — the first compiles precisely when the feature is OFF —
 * so anything that only looks for the name reads them backwards.
 */
function parseCfgPredicate(text) {
  let at = 0;
  const skipSpace = () => {
    while (at < text.length && /\s/.test(text[at])) at++;
  };

  function parseNode() {
    skipSpace();
    const start = at;
    while (at < text.length && /[A-Za-z0-9_]/.test(text[at])) at++;
    const ident = text.slice(start, at);
    skipSpace();

    if (text[at] === '(') {
      at++;
      const children = [];
      for (;;) {
        skipSpace();
        if (at >= text.length || text[at] === ')') {
          at++;
          break;
        }
        children.push(parseNode());
        skipSpace();
        if (text[at] === ',') at++;
      }
      return { kind: ident, children };
    }

    if (text[at] === '=') {
      at++;
      skipSpace();
      const quote = text[at];
      if (quote === '"' || quote === "'") {
        at++;
        const from = at;
        while (at < text.length && text[at] !== quote) at++;
        const value = text.slice(from, at);
        at++;
        return { kind: 'atom', key: ident, value };
      }
    }

    // A bare atom such as `unix` or `test`: true or false on its own terms,
    // and never a statement about a feature.
    return { kind: 'atom', key: ident };
  }

  return parseNode();
}

/**
 * Does this predicate being TRUE prove `feature` is enabled?
 *
 * Conservative by construction — it answers "no" whenever it cannot prove
 * "yes", which is the safe direction here: a gate wrongly believed to protect a
 * namespace is what removes reachable controllers from the denominator.
 *
 *  - `all(...)`: true means every child is true, so ONE child requiring the
 *    feature is enough.
 *  - `any(...)`: true means at least one child is true, so the feature is
 *    implied only if EVERY branch requires it. `any(feature = "x", unix)` is
 *    satisfied on unix with `x` off.
 *  - `not(...)`: proves nothing about the feature being on, and
 *    `not(feature = "x")` is true exactly when it is off.
 */
function cfgProvesFeature(node, feature) {
  if (!node) return false;
  switch (node.kind) {
    case 'atom':
      return node.key === 'feature' && node.value === feature;
    case 'all':
      return node.children.some((child) => cfgProvesFeature(child, feature));
    case 'any':
      return node.children.length > 0 && node.children.every((child) => cfgProvesFeature(child, feature));
    default:
      // `not`, and anything unrecognised, prove nothing.
      return false;
  }
}

/**
 * The `#[...]` attribute bodies attached to the item starting at `index`.
 *
 * Walks backwards over stacked attributes and any doc comments between them,
 * matching brackets rather than reading a line at a time — a `#[cfg(all(
 * feature = "x",
 * unix))]` split across lines is one attribute, and a line-based scan would
 * see only its last line and parse nothing.
 */
function attributesBefore(text, index) {
  const attributes = [];
  let end = index;

  for (;;) {
    let cursor = end - 1;
    for (;;) {
      while (cursor >= 0 && /\s/.test(text[cursor])) cursor--;
      const lineStart = text.lastIndexOf('\n', cursor) + 1;
      if (/^\s*\/\//.test(text.slice(lineStart, cursor + 1))) {
        cursor = lineStart - 1;
        continue;
      }
      break;
    }

    if (cursor < 0 || text[cursor] !== ']') break;
    let depth = 0;
    let open = cursor;
    for (; open >= 0; open--) {
      if (text[open] === ']') depth++;
      else if (text[open] === '[') {
        depth--;
        if (depth === 0) break;
      }
    }
    if (open <= 0 || text[open - 1] !== '#') break;

    attributes.push(text.slice(open + 1, cursor));
    end = open - 1;
  }

  return attributes;
}

/**
 * Is `file` reachable only when `feature` is enabled?
 *
 * `#[cfg]` sits on the `mod` declaration in the PARENT, never in the file
 * itself, so this walks upward: `src/openhuman/test_support/schemas.rs` is
 * reached through `mod schemas;` in `test_support/mod.rs` and then through
 * `pub mod test_support;` in `openhuman/mod.rs` — and only the second carries
 * the gate. A file pulled in by `include!` has no `mod` of its own and simply
 * contributes nothing at its own level, which is why a missing declaration is
 * not an error here; one gated ancestor anywhere on the chain is enough.
 */
function moduleGateProves(file, feature) {
  let segments = path.relative(ROOT, file).split(path.sep);

  while (segments.length > 1) {
    const base = segments[segments.length - 1].replace(/\.rs$/, '');
    const isModFile = base === 'mod';
    // A `mod.rs` IS its directory's module, so its declaration lives one level
    // further up and under the directory's name.
    const name = isModFile ? segments[segments.length - 2] : base;
    const parentDir = isModFile ? segments.slice(0, -2) : segments.slice(0, -1);
    const declaringFile = path.join(ROOT, ...parentDir, 'mod.rs');

    if (fs.existsSync(declaringFile)) {
      const text = read(declaringFile);
      const declaration = new RegExp(`^[ \\t]*(?:pub(?:\\([^)]*\\))?[ \\t]+)?mod[ \\t]+${name}[ \\t]*;`, 'gm');
      // EVERY declaration has to imply the feature, not merely the first one.
      // A module may be declared more than once under mutually exclusive
      // predicates —
      //
      //   #[cfg(feature = "x")]      mod test_support;
      //   #[cfg(not(feature = "x"))] mod test_support;
      //
      // — and it then exists in BOTH configurations. Reading only the first
      // declaration sees the gate and calls the module absent when it is
      // always present, so the test is "does every path to this module require
      // the feature", and one unproven declaration settles it.
      const declarations = [...text.matchAll(declaration)];
      if (declarations.length > 0) {
        const everyPathRequiresIt = declarations.every((found) =>
          attributesBefore(text, found.index).some((attribute) => {
            const inner = attribute.match(/^\s*cfg\s*\(([\s\S]*)\)\s*$/)?.[1];
            return Boolean(inner) && cfgProvesFeature(parseCfgPredicate(inner), feature);
          }),
        );
        // Not proving it here is not a verdict: an ancestor `mod` may still be
        // gated, and that would make the file unreachable all the same.
        if (everyPathRequiresIt) return true;
      }
    }

    segments = parentDir;
    if (segments[segments.length - 1] === 'src') break;
  }

  return false;
}

/**
 * The gates cargo has ON in the build `scripts/test-rust-e2e.sh` produces.
 *
 * Resolved from the same two files that script reads, and through the feature
 * graph rather than by direct membership: a gate can be enabled transitively
 * (`documents = ["modules", …]`), and reading such a gate as OFF would let an
 * exclusion look earned when cargo compiles the family in.
 *
 * Both files are REQUIRED. Without them this gate cannot say which controllers
 * are reachable, and an exclusion nothing verifies is precisely the silent hole
 * `UNREACHABLE_NAMESPACES` exists to close.
 */
function measuredFeatures() {
  for (const file of [CORE_MANIFEST, PRODUCT_FEATURES_FILE]) {
    if (fs.existsSync(file)) continue;
    console.error(
      `check-domain-e2e-coverage: ${path.relative(ROOT, file) || file} not found under ${ROOT}.\n` +
        'The gate resolves which controllers are reachable in the configuration it measures from\n' +
        'Cargo.toml and scripts/ci/product-features.txt, so it must run from the repository root.',
    );
    process.exit(2);
  }
  const graph = parseCoreFeatureGraph(read(CORE_MANIFEST));
  // Guard the guard. An empty graph is what a moved `[features]` table or a
  // parser regression looks like, and it does not fail — it quietly reports
  // every gate as OFF, which is the answer that makes every exclusion look
  // earned. Refuse instead: this check is only worth having if it can be wrong.
  if (!graph.has('default')) {
    console.error(
      `check-domain-e2e-coverage: no \`[features] default\` in ${path.relative(ROOT, CORE_MANIFEST)}.\n` +
        'Without it the measured feature set cannot be resolved, and every exclusion in\n' +
        'UNREACHABLE_NAMESPACES would be accepted unchecked.',
    );
    process.exit(2);
  }
  const product = parseProductFeatures(read(PRODUCT_FEATURES_FILE));
  return { graph, enabled: resolveEnabledFeatures(graph, ['default', ...product]) };
}

/**
 * The three ways an `UNREACHABLE_NAMESPACES` entry can stop being true.
 *
 * Returns one message per problem; empty means every exclusion is still earned.
 * This exists because the MODULES comment above is right — a list you must
 * remember to maintain is a list that silently stops covering things — and an
 * exclusion that rots is worse than a stale MODULES line: it does not merely
 * fail to measure something, it deletes a real obligation from the denominator
 * and reports the smaller world as success.
 */
function checkExclusions(discovered, declaringFiles, labelForNamespace) {
  const { graph, enabled } = measuredFeatures();
  const problems = [];

  // (0) The gate no longer exists. A rename that updated the `#[cfg]` sites but
  // missed this table leaves a feature name no `[features]` entry declares —
  // absent from the graph, so absent from `enabled`, which check (1) below
  // reads as "safely disabled". The namespace still exists and is not in
  // MODULES, so neither of the other checks fires either, and the renamed
  // family's controllers leave the denominator in silence. "Not enabled" and
  // "not a gate at all" have to be different answers.
  const undeclared = Object.entries(UNREACHABLE_NAMESPACES)
    .filter(([, entry]) => !graph.has(entry.feature))
    .map(([namespace, entry]) => `${namespace} (gated on "${entry.feature}")`)
    .sort();
  if (undeclared.length > 0) {
    problems.push(
      `${undeclared.length} excluded namespace(s) name a feature the \`[features]\` table does not declare: ${undeclared.join(', ')}.` +
        '\nA renamed or deleted gate reads as disabled here, which would accept the exclusion unchecked.' +
        '\nPoint the UNREACHABLE_NAMESPACES entry at the current gate name, or drop it.',
    );
  }

  // (1) The gate came back. If the feature is enabled in the measured build,
  // the controllers dispatch and excluding them hides work that is now real.
  const reachable = Object.entries(UNREACHABLE_NAMESPACES)
    .filter(([, entry]) => enabled.has(entry.feature))
    .map(([namespace, entry]) => `${namespace} (gated on "${entry.feature}")`)
    .sort();
  if (reachable.length > 0) {
    problems.push(
      `${reachable.length} excluded namespace(s) are reachable in the measured configuration: ${reachable.join(', ')}.` +
        '\nTheir gate is enabled by `[features] default` or scripts/ci/product-features.txt, so their controllers' +
        '\ndo dispatch and must be covered. Drop the UNREACHABLE_NAMESPACES entry.',
    );
  }

  // (2) The namespace is gone, or discovery stopped seeing it. Same failure
  // `declaredButMissing` catches for MODULES, applied to the other list.
  const missing = Object.keys(UNREACHABLE_NAMESPACES)
    .filter((namespace) => !discovered.has(namespace))
    .sort();
  if (missing.length > 0) {
    problems.push(
      `UNREACHABLE_NAMESPACES names ${missing.length} namespace(s) with no discovered controllers: ${missing.join(', ')}.` +
        '\nEither the namespace was removed (drop the entry) or schema discovery has stopped seeing it' +
        '\n(fix SCHEMA_ROOTS / the match).',
    );
  }

  // (2b) The `#[cfg]` itself is gone. Every check here can pass while the Rust
  // gate that made the namespace unreachable has been deleted: the feature is
  // still declared (0), still disabled (1), the namespace is still discovered
  // (2) and still absent from MODULES (3) — but its module now compiles
  // unconditionally and its controllers dispatch. An exclusion is a claim about
  // the source, so it is checked against the source: every file declaring the
  // namespace must sit behind a `#[cfg(feature = …)]` naming the claimed gate.
  const ungated = [];
  for (const [namespace, entry] of Object.entries(UNREACHABLE_NAMESPACES)) {
    const files = declaringFiles.get(namespace);
    if (!files) continue; // Already reported by (2); nothing to check against.
    const unguarded = [...files]
      .filter((file) => !moduleGateProves(file, entry.feature))
      .map((file) => path.relative(ROOT, file))
      .sort();
    if (unguarded.length > 0) {
      ungated.push(`${namespace} — ${unguarded.join(', ')} (expected \`#[cfg(feature = "${entry.feature}")]\`)`);
    }
  }
  if (ungated.length > 0) {
    problems.push(
      `${ungated.length} excluded namespace(s) are no longer behind the gate they claim:\n  ${ungated.join('\n  ')}` +
        '\nThe module compiles unconditionally now, so its controllers dispatch and must be covered.' +
        '\nRestore the `#[cfg]`, or drop the UNREACHABLE_NAMESPACES entry.',
    );
  }

  // (3) Naming one namespace in both lists is a contradiction: MODULES asks for
  // a coverage row, UNREACHABLE_NAMESPACES says there is nothing to cover.
  // Unchecked, the exclusion wins and the namespace vanishes from the report
  // without a word — which is how a MODULES entry stops meaning anything.
  const contradictory = Object.keys(UNREACHABLE_NAMESPACES)
    .filter((namespace) => labelForNamespace.has(namespace))
    .sort();
  if (contradictory.length > 0) {
    problems.push(
      `${contradictory.length} namespace(s) appear in BOTH MODULES and UNREACHABLE_NAMESPACES: ${contradictory.join(', ')}.` +
        '\nMODULES asks for a coverage row; UNREACHABLE_NAMESPACES says there is nothing to cover. Resolve one.',
    );
  }

  return problems;
}

const invoked = collectInvokedMethods();
const { methodsByNamespace: schemas, filesByNamespace: schemaFiles } = collectSchemaMethods();

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

const exclusionProblems = checkExclusions(schemas, schemaFiles, labelForNamespace);

// Reported below rather than dropped in silence: an excluded namespace is a
// claim ("nothing here can be dispatched"), and a claim the report does not
// show is a claim nobody reviews.
const excluded = [...schemas]
  .filter(([namespace]) => Object.hasOwn(UNREACHABLE_NAMESPACES, namespace))
  .map(([namespace, methods]) => ({ namespace, count: methods.size, ...UNREACHABLE_NAMESPACES[namespace] }))
  .sort((a, b) => a.namespace.localeCompare(b.namespace));

// One row per namespace that actually exists, grouped where MODULES says so.
const rows = new Map();
for (const [namespace, methods] of schemas) {
  if (Object.hasOwn(UNREACHABLE_NAMESPACES, namespace)) continue;
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

if (excluded.length > 0) {
  const totalExcluded = excluded.reduce((sum, entry) => sum + entry.count, 0);
  console.log('');
  console.log(
    `Excluded ${totalExcluded} controller(s) in ${excluded.length} namespace(s) as unreachable in the measured` +
      ' configuration (`[features] default` + scripts/ci/product-features.txt):',
  );
  for (const entry of excluded) {
    console.log(`  ${entry.namespace} (${entry.count}) — compiled out by \`${entry.feature}\`: ${entry.reason}`);
  }
}

if (exclusionProblems.length > 0) {
  failed = true;
  for (const problem of exclusionProblems) console.error(`\n${problem}`);
}

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
