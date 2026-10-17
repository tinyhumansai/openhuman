#!/usr/bin/env node
// Enforces the crate chain: core -> embed -> tinyhumans -> rpc -> app/cli/tui.
//
// Each OpenHuman crate may name exactly one OpenHuman crate among its NORMAL
// dependencies — the layer directly below it — and the hosts (desktop app, CLI,
// TUI) name `openhuman-rpc` only. Dev-dependencies and build-dependencies are
// exempt: the CLI's root integration tests reach into the core on purpose, and
// none of that reaches a shipped binary.
//
// A second check covers what a manifest cannot show: host source must not
// reach core internals by path. The layers above embed get core internals
// through embed's doc-hidden `__host` list (`core_host` inside rpc); hosts get
// the curated facade (`openhuman_rpc::embed`, `openhuman_rpc::tinyhumans`) and
// nothing else. So `__host`, `core_host` and `openhuman_core::` in
// `crates/openhuman-{app,cli,tui}/src` fail the check.
//
// A third check covers the facades. No layer re-exports the layer below
// wholesale (`pub use openhuman_embed as embed`, `pub use openhuman_tinyhumans
// as tinyhumans`, a glob, or `pub use …::embed;`): each one names the items it
// passes up, so `__host` cannot leak to a host through a re-exported crate.
// `__host` itself may be forwarded by tinyhumans (rpc's only route to it, doc
// hidden) and must stay a private binding inside rpc; any `pub use` of `__host` or
// `core_host` in rpc, or a reach for either through `openhuman_rpc::` from a
// host, root test or example, fails.
//
// Usage: check-crate-chain.mjs [repo-root]
// Exit 0 when the chain holds, 1 on a violation, 2 when the inputs could not
// be read or parsed (the check refuses to pass vacuously).

import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { LINKED_MODULES_FEATURE as LINKED_FEATURE, parseFeatureTable, stripComments } from '../lib/feature-forwarding.mjs';

/**
 * Package name -> the OpenHuman packages it may name as normal dependencies.
 * A package missing from this table is an unknown OpenHuman crate and fails
 * the check until someone places it in the chain on purpose.
 */
export const CHAIN = {
  openhuman: [],
  'openhuman-embed': ['openhuman'],
  'openhuman-tinyhumans': ['openhuman-embed'],
  'openhuman-rpc': ['openhuman-tinyhumans'],
  'openhuman-app': ['openhuman-rpc'],
  // The CLI alone may also name the local-development bridge that links module
  // implementation crates; `checkLinkedBridgeGate` pins HOW (optional, behind
  // the non-product `linked-modules` feature).
  'openhuman-cli': ['openhuman-rpc', 'openhuman-modules-linked'],
  'openhuman-tui': ['openhuman-rpc'],
  'openhuman-modules-linked': ['openhuman-rpc'],
};

/** The local-development crate that links module implementations. */
export const LINKED_BRIDGE = 'openhuman-modules-linked';
// The only feature allowed to enable LINKED_BRIDGE (never a product gate) is
// LINKED_FEATURE, defined in scripts/lib/feature-forwarding.mjs. It is imported
// rather than spelled here because scripts/ci/ is scanned for that name by
// check-ci-module-source.mjs.

/** The host crates whose `src/` must not reach core internals by path. */
export const HOST_SOURCE_DIRS = [
  'crates/openhuman-app/src',
  'crates/openhuman-cli/src',
  'crates/openhuman-tui/src',
];

/** Paths a host must never name (see the header). */
export const FORBIDDEN_HOST_PATTERNS = [
  { name: '__host', regex: /\b__host\b/ },
  { name: 'core_host', regex: /\bcore_host\b/ },
  { name: 'openhuman_core::', regex: /\bopenhuman_core::/ },
];

/** Layer crates whose `src/` must not re-export the layer below wholesale. */
export const LAYER_SOURCE_DIRS = [
  'crates/openhuman-embed/src',
  'crates/openhuman-tinyhumans/src',
  'crates/openhuman-rpc/src',
];

/** Wholesale re-exports of a lower layer (matched across lines, comments stripped). */
export const WHOLESALE_REEXPORT_PATTERNS = [
  {
    name: 'pub use openhuman_embed as …',
    regex: /\bpub(?:\([^)]*\))?\s+use\s+openhuman_embed\s+as\b/,
  },
  {
    name: 'pub use openhuman_tinyhumans as …',
    regex: /\bpub(?:\([^)]*\))?\s+use\s+openhuman_tinyhumans\s+as\b/,
  },
  {
    name: 'pub use openhuman_embed::*',
    regex: /\bpub(?:\([^)]*\))?\s+use\s+openhuman_(?:embed|tinyhumans)(?:::embed)?::\*/,
  },
  {
    name: 'pub use openhuman_embed::{self, *} (grouped wholesale)',
    regex:
      /\bpub(?:\([^)]*\))?\s+use\s+openhuman_(?:embed|tinyhumans)(?:::embed)?::\{[^}]*(?:\bself\b|\*)/,
  },
  {
    name: 'pub use openhuman_embed / openhuman_tinyhumans (the bare crate)',
    regex: /\bpub(?:\([^)]*\))?\s+use\s+openhuman_(?:embed|tinyhumans)\s*;/,
  },
  {
    name: 'pub use openhuman_tinyhumans::{embed, …} (grouped)',
    regex: /\bpub(?:\([^)]*\))?\s+use\s+openhuman_tinyhumans::\{[^}]*\bembed\b(?!\s*::)[^}]*\}/,
  },
  {
    name: 'pub use openhuman_tinyhumans::embed (the crate, not a list)',
    regex: /\bpub(?:\([^)]*\))?\s+use\s+openhuman_tinyhumans::embed\s*(?:as\s+\w+\s*)?;/,
  },
];

/** Embed must not re-export the core wholesale either. */
export const CORE_WHOLESALE_REEXPORT_PATTERNS = [
  {
    name: 'pub use openhuman_core (the bare crate, an alias or a glob)',
    regex: /\bpub(?:\([^)]*\))?\s+use\s+openhuman_core\s*(?:;|as\b|::\*|::\{[^}]*(?:\bself\b|\*))/,
  },
];

/** `pub use` of the internal list from rpc (it may only be `pub(crate)`). */
export const RPC_INTERNAL_REEXPORT_PATTERNS = [
  {
    name: 'pub use of anything under __host / core_host (bar unwrap_rpc)',
    regex: /\bpub\s+use\b(?!\s+[^;{},]*::unwrap_rpc\s*;)[^;]*\b(?:__host|core_host)\b/,
  },
];

/** Hosts, root tests and examples must not reach the internal list through rpc. */
export const HOST_RPC_INTERNAL_PATTERNS = [
  { name: '__host / core_host (any path or alias)', regex: /\b(?:__host|core_host)\b/ },
];

/**
 * Blank out Rust comments and string literals (contents only), keeping line
 * structure, so a banned path quoted in a doc, log message or fixture is not
 * mistaken for code. Handles nested block comments, escapes and raw strings.
 * Optional literal preservation and code-whitespace compaction support type
 * signatures; default masking is unchanged. Compaction leaves literal contents intact.
 * preserveTokenBoundaries retains separators that prevent lexical token merging.
 */
export function stripRustComments(text, { preserveLiterals = false, compact = false, preserveTokenBoundaries = false } = {}) {
  let out = '';
  let i = 0;
  let pendingBoundary = false;
  const word = char => /[\p{ID_Continue}]/u.test(char);
  // Rust punctuation spacing can distinguish macro arms, including a third
  // character completing an operator. Retain all separated punctuation pairs.
  const punctuation = new Set("=<>!~+-*/%^&|@.,;:#$?'");
  const append = chunk => {
    if (!chunk) return;
    if (compact && preserveTokenBoundaries && pendingBoundary && out) {
      const tail = out.slice(-2);
      const left = [...tail].at(-1);
      const right = String.fromCodePoint(chunk.codePointAt(0));
      if ((word(left) && word(right)) ||
          (word(left) && ['"', "'", '#'].includes(right)) ||
          (['"', "'", '#'].includes(left) && word(right)) ||
          (left === '#' && right === '"') ||
          (left === '.' && word(right)) ||
          (/[0-9]/.test(left) && right === '.') ||
          (/[eE]/.test(left) && ['+', '-'].includes(right)) ||
          (['+', '-'].includes(left) && /[0-9]/.test(right)) ||
          (punctuation.has(left) && punctuation.has(right))) out += ' ';
    }
    out += chunk;
    pendingBoundary = false;
  };
  const blank = chunk => {
    if (compact) {
      pendingBoundary = true;
      return '';
    }
    return chunk.replace(/[^\n]/g, ' ');
  };
  while (i < text.length) {
    const rest = text.slice(i);
    let m;
    if (rest.startsWith('//')) {
      const end = text.indexOf('\n', i);
      const stop = end === -1 ? text.length : end;
      append(blank(text.slice(i, stop)));
      i = stop;
    } else if (rest.startsWith('/*')) {
      let depth = 0;
      let j = i;
      while (j < text.length) {
        if (text.startsWith('/*', j)) {
          depth++;
          j += 2;
        } else if (text.startsWith('*/', j)) {
          depth--;
          j += 2;
          if (depth === 0) break;
        } else j++;
      }
      append(blank(text.slice(i, j)));
      i = j;
    } else if ((m = /^b?r(#*)"/.exec(rest))) {
      const close = '"' + m[1];
      const end = text.indexOf(close, i + m[0].length);
      const stop = end === -1 ? text.length : end + close.length;
      append(preserveLiterals ? text.slice(i, stop) : blank(text.slice(i, stop)));
      i = stop;
    } else if ((m = /^b?'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]+\}|.)|[^\\'])'/.exec(rest))) {
      append(preserveLiterals ? m[0] : blank(m[0]));
      i += m[0].length;
    } else if (rest.startsWith('"') || rest.startsWith('b"')) {
      let j = i + (rest.startsWith('b') ? 2 : 1);
      while (j < text.length && text[j] !== '"') j += text[j] === '\\' ? 2 : 1;
      append(preserveLiterals ? text.slice(i, j + 1) : blank(text.slice(i, j + 1)));
      i = j + 1;
    } else {
      const char = String.fromCodePoint(text.codePointAt(i));
      if (compact && /\s/.test(char)) pendingBoundary = true;
      else append(char);
      i += char.length;
    }
  }
  return out;
}

/** `{ file, line, pattern }` for each multi-line pattern match in one source text. */
export function findPatternHits(text, file, patterns) {
  const code = stripRustComments(text);
  const hits = [];
  for (const { name, regex } of patterns) {
    const global = new RegExp(regex.source, 'g');
    for (const m of code.matchAll(global)) {
      hits.push({ file, line: code.slice(0, m.index).split('\n').length, pattern: name });
    }
  }
  return hits;
}

/** Whether a package name belongs to this repository's OpenHuman crates. */
export function isOpenhumanPackage(name) {
  return name === 'openhuman' || name.startsWith('openhuman-');
}

function tableBody(text, start) {
  const rest = text.slice(start);
  const next = rest.search(/^[ \t]*\[/m);
  return next === -1 ? rest : rest.slice(0, next);
}

/** `[package] name = "..."`, or null. */
export function parsePackageName(toml) {
  const text = stripComments(toml);
  const header = text.match(/^[ \t]*\[package\][ \t]*$/m);
  if (!header) return null;
  const body = tableBody(text, header.index + header[0].length);
  const name = body.match(/^[ \t]*name[ \t]*=[ \t]*"([^"]+)"/m);
  return name ? name[1] : null;
}

/** Root `[workspace] members = [...]`. */
export function parseWorkspaceMembers(toml) {
  const text = stripComments(toml);
  const header = text.match(/^[ \t]*\[workspace\][ \t]*$/m);
  if (!header) return [];
  const body = tableBody(text, header.index + header[0].length);
  const at = body.search(/^[ \t]*members[ \t]*=[ \t]*\[/m);
  if (at === -1) return [];
  const open = body.indexOf('[', at);
  const close = body.indexOf(']', open);
  return [...body.slice(open + 1, close).matchAll(/"([^"]+)"/g)].map(m => m[1]);
}

/** One dependency entry's value text (inline table or string), joined across lines. */
function entryValue(body, valueStart) {
  if (body[valueStart] !== '{') {
    const end = body.indexOf('\n', valueStart);
    return end === -1 ? body.slice(valueStart) : body.slice(valueStart, end);
  }
  let depth = 0;
  for (let i = valueStart; i < body.length; i++) {
    if (body[i] === '{') depth++;
    else if (body[i] === '}') {
      depth--;
      if (depth === 0) return body.slice(valueStart, i + 1);
    }
  }
  return body.slice(valueStart);
}

/**
 * Dependencies declared in one dependency table body, as
 * `{ key, package, workspace }`: `package` is the `package = "..."` rename when
 * given, else the key; `workspace` marks `key.workspace = true` /
 * `{ workspace = true }`, whose package the root `[workspace.dependencies]`
 * table decides.
 */
export function parseDependencyEntries(body) {
  const entries = [];
  for (const match of body.matchAll(/^[ \t]*([A-Za-z0-9_-]+)((?:\.workspace)?)[ \t]*=[ \t]*/gm)) {
    const key = match[1];
    const value = entryValue(body, match.index + match[0].length);
    const pkg = value.match(/package[ \t]*=[ \t]*"([^"]+)"/);
    const workspace = match[2] === '.workspace' || /workspace[ \t]*=[ \t]*true/.test(value);
    entries.push({ key, package: pkg ? pkg[1] : key, workspace });
  }
  return entries;
}

/**
 * Every NORMAL dependency of a manifest: `[dependencies]`, the
 * `[target.'…'.dependencies]` tables, and `[dependencies.<name>]` sub-tables.
 * Dev- and build-dependencies are skipped.
 */
export function parseNormalDependencies(toml) {
  const text = stripComments(toml);
  const deps = [];
  for (const header of text.matchAll(/^[ \t]*\[([^\]\n]+)\][ \t]*$/gm)) {
    const table = header[1].trim();
    const body = tableBody(text, header.index + header[0].length);
    if (table === 'dependencies' || /^target\..+\.dependencies$/.test(table)) {
      deps.push(...parseDependencyEntries(body));
      continue;
    }
    const sub = table.match(/^(?:target\..+\.)?dependencies\.([A-Za-z0-9_-]+)$/);
    if (sub) {
      const pkg = body.match(/^[ \t]*package[ \t]*=[ \t]*"([^"]+)"/m);
      const workspace = /^[ \t]*workspace[ \t]*=[ \t]*true/m.test(body);
      deps.push({ key: sub[1], package: pkg ? pkg[1] : sub[1], workspace });
    }
  }
  return deps;
}

/** Root `[workspace.dependencies]` as key -> package name. */
export function parseWorkspaceDependencies(toml) {
  const text = stripComments(toml);
  const header = text.match(/^[ \t]*\[workspace\.dependencies\][ \t]*$/m);
  if (!header) return new Map();
  const body = tableBody(text, header.index + header[0].length);
  return new Map(parseDependencyEntries(body).map(entry => [entry.key, entry.package]));
}

/**
 * Violations of {@link CHAIN} for one manifest. Each is
 * `{ crate, dependency, reason }`.
 */
export function checkManifestEdges({ crate, deps, workspaceDeps = new Map(), chain = CHAIN }) {
  const violations = [];
  if (!Object.prototype.hasOwnProperty.call(chain, crate)) {
    violations.push({
      crate,
      dependency: null,
      reason: 'unknown OpenHuman crate: add it to CHAIN',
    });
    return violations;
  }
  const allowed = new Set(chain[crate]);
  for (const dep of deps) {
    const pkg = dep.workspace ? (workspaceDeps.get(dep.key) ?? dep.package) : dep.package;
    if (!isOpenhumanPackage(pkg)) continue;
    if (!allowed.has(pkg)) {
      violations.push({
        crate,
        dependency: pkg,
        reason: allowed.size
          ? `may only depend on ${[...allowed].join(', ')} among OpenHuman crates`
          : 'may not depend on any OpenHuman crate',
      });
    }
  }
  return violations;
}

/**
 * How the CLI may depend on the linked-modules bridge: only as an OPTIONAL
 * normal dependency enabled by the `linked-modules` feature and by nothing
 * else (not `default`, not another gate). Returns `{ crate, dependency, reason }`
 * violations; a CLI that does not name the bridge at all has none.
 */
export function checkLinkedBridgeGate(cliToml, crate = 'openhuman-cli') {
  const violations = [];
  const fail = reason => violations.push({ crate, dependency: LINKED_BRIDGE, reason });
  const text = stripComments(cliToml);
  const dep = parseNormalDependencies(cliToml).find(entry => entry.package === LINKED_BRIDGE);
  if (!dep) return violations;
  const declaration = text.match(
    new RegExp(`^[ \\t]*${dep.key}[ \\t]*=[ \\t]*\\{[^}]*\\}`, 'm')
  );
  if (!declaration || !/optional[ \t]*=[ \t]*true/.test(declaration[0])) {
    fail('must be an `optional = true` dependency so a default build does not link it');
  }
  const features = parseFeatureTable(cliToml);
  const gate = features.get(LINKED_FEATURE);
  if (!gate || !gate.includes(`dep:${dep.key}`)) {
    fail(`the \`${LINKED_FEATURE}\` feature must enable it with \`dep:${dep.key}\``);
  }
  for (const [name, items] of features) {
    if (name === LINKED_FEATURE) continue;
    if (items.some(item => item === LINKED_FEATURE || item === `dep:${dep.key}` || item.startsWith(`${dep.key}/`))) {
      fail(`feature \`${name}\` enables the linked-module bridge; only \`${LINKED_FEATURE}\` may`);
    }
  }
  return violations;
}

/** `{ file, line, pattern }` for every forbidden path in one source text. */
export function findForbiddenPaths(text, file, patterns = FORBIDDEN_HOST_PATTERNS) {
  const hits = [];
  text.split(/\r?\n/).forEach((line, index) => {
    for (const { name, regex } of patterns) {
      if (regex.test(line)) hits.push({ file, line: index + 1, pattern: name });
    }
  });
  return hits;
}

function rustFiles(dir) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) out.push(...rustFiles(path));
    else if (entry.endsWith('.rs')) out.push(path);
  }
  return out;
}

/** Run both checks against a repository checkout. */
export function checkRepository(root) {
  const rootToml = readFileSync(join(root, 'Cargo.toml'), 'utf8');
  const members = parseWorkspaceMembers(rootToml);
  if (members.length === 0) {
    throw new Error('parsed zero workspace members from the root Cargo.toml');
  }
  const workspaceDeps = parseWorkspaceDependencies(rootToml);
  // The desktop shell is its own Cargo world, excluded from the workspace.
  const manifestDirs = [...members, 'crates/openhuman-app'];
  const edgeViolations = [];
  const checked = [];
  for (const dir of manifestDirs) {
    const manifest = join(root, dir, 'Cargo.toml');
    if (!existsSync(manifest)) throw new Error(`missing manifest ${relative(root, manifest)}`);
    const toml = readFileSync(manifest, 'utf8');
    const crate = parsePackageName(toml);
    if (!crate) throw new Error(`no [package] name in ${relative(root, manifest)}`);
    if (!isOpenhumanPackage(crate)) continue;
    checked.push(crate);
    edgeViolations.push(
      ...checkManifestEdges({ crate, deps: parseNormalDependencies(toml), workspaceDeps })
    );
    if (crate === 'openhuman-cli') edgeViolations.push(...checkLinkedBridgeGate(toml, crate));
  }
  const sourceViolations = [];
  for (const dir of HOST_SOURCE_DIRS) {
    const abs = join(root, dir);
    if (!existsSync(abs)) throw new Error(`missing host source directory ${dir}`);
    for (const file of rustFiles(abs)) {
      sourceViolations.push(
        ...findForbiddenPaths(readFileSync(file, 'utf8'), relative(root, file))
      );
    }
  }
  const facadeViolations = [];
  for (const dir of LAYER_SOURCE_DIRS) {
    const abs = join(root, dir);
    if (!existsSync(abs)) throw new Error(`missing layer source directory ${dir}`);
    const patterns = dir.includes('openhuman-rpc')
      ? [...WHOLESALE_REEXPORT_PATTERNS, ...RPC_INTERNAL_REEXPORT_PATTERNS]
      : dir.includes('openhuman-embed')
        ? CORE_WHOLESALE_REEXPORT_PATTERNS
        : WHOLESALE_REEXPORT_PATTERNS;
    for (const file of rustFiles(abs)) {
      facadeViolations.push(
        ...findPatternHits(readFileSync(file, 'utf8'), relative(root, file), patterns)
      );
    }
  }
  const reachDirs = [
    ...HOST_SOURCE_DIRS,
    'tests',
    'examples',
    ...HOST_SOURCE_DIRS.map(d => d.replace(/src$/, 'tests')),
  ];
  for (const dir of reachDirs) {
    const abs = join(root, dir);
    if (!existsSync(abs)) continue;
    for (const file of rustFiles(abs)) {
      facadeViolations.push(
        ...findPatternHits(
          readFileSync(file, 'utf8'),
          relative(root, file),
          HOST_RPC_INTERNAL_PATTERNS
        )
      );
    }
  }
  return { checked, edgeViolations, sourceViolations, facadeViolations };
}

export function formatReport({ checked, edgeViolations, sourceViolations, facadeViolations = [] }) {
  const lines = [`Crate chain: core -> embed -> tinyhumans -> rpc -> app/cli/tui`];
  lines.push(`Checked ${checked.length} manifests: ${checked.join(', ')}`);
  if (edgeViolations.length > 0) {
    lines.push('', 'Normal-dependency edges that break the chain:');
    for (const v of edgeViolations) {
      lines.push(`  - ${v.crate} -> ${v.dependency ?? '?'}: ${v.reason}`);
    }
    lines.push(
      '',
      'Reach the layer below through its curated facade instead. Hosts name',
      '`openhuman-rpc` only (`openhuman_rpc::embed`, `openhuman_rpc::tinyhumans`);',
      'a test-only need belongs in [dev-dependencies].'
    );
  }
  if (sourceViolations.length > 0) {
    lines.push('', 'Host source reaching core internals by path:');
    for (const v of sourceViolations) lines.push(`  - ${v.file}:${v.line}: ${v.pattern}`);
    lines.push(
      '',
      'Hosts use the curated facade (`openhuman_rpc::embed::…`). `__host` /',
      '`core_host` are internal to the library layers; add what the host needs to',
      "embed's public facade instead."
    );
  }
  if (facadeViolations.length > 0) {
    lines.push('', 'Facades that re-export a lower layer wholesale or leak `__host`:');
    for (const v of facadeViolations) lines.push(`  - ${v.file}:${v.line}: ${v.pattern}`);
    lines.push(
      '',
      'Each layer re-exports a curated `pub use` list of the items the layers above',
      'use, never the crate below it. `__host` stays out of every public path in rpc.'
    );
  }
  if (
    edgeViolations.length === 0 &&
    sourceViolations.length === 0 &&
    facadeViolations.length === 0
  ) {
    lines.push('OK: every crate depends only on the layer below it.');
  }
  return lines.join('\n');
}

function main() {
  const root = resolve(
    process.argv[2] ?? resolve(dirname(fileURLToPath(import.meta.url)), '../..')
  );
  let result;
  try {
    result = checkRepository(root);
  } catch (err) {
    console.error(`check-crate-chain: could not check ${root}: ${err.message}`);
    process.exit(2);
  }
  const report = formatReport(result);
  const ok =
    result.edgeViolations.length === 0 &&
    result.sourceViolations.length === 0 &&
    result.facadeViolations.length === 0;
  (ok ? console.log : console.error)(report);
  process.exit(ok ? 0 : 1);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main();
}
