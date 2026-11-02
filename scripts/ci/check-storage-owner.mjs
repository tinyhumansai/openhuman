#!/usr/bin/env node
// Storage-engine gate — tinyhumansai/openhuman#8131, part of #442.
//
// tinystoragedrivers is the only crate family allowed to own a storage
// backend. This check reads `cargo metadata --all-features` for the root
// workspace and for `crates/openhuman-app` and fails when
//
//   Rule 1: a first-party package (path or git source, not a registry dep)
//           that is NOT a tsd crate and NOT a composition root reaches a
//           forbidden engine crate through any normal or build dependency.
//           The walk stops at composition roots (what a root reaches is its
//           own), and the exact edges in ENGINE_EXCEPTIONS are excused; a
//           stale exception fails.
//
//   Rule 2: a first-party package not in tsd is named  `*-sqlite`,
//           `*-mongodb` or `*-file`.
//
//   Rule 3: a tinystoragedrivers* package in the resolved graph is not a
//           path under `vendor/tinystoragedrivers/` (anchored to this repo).
//
//   Rule 4: a dependency names tinystoragedrivers by a git tag that differs
//           from the vendored checkout's version.
//
// Read-only readers of external databases are not storage and are listed in
// ALLOWLIST with a reason.
//
// Usage: check-storage-owner.mjs [repo-root] [--metadata <file.json>]...
// With --metadata the given `cargo metadata` JSON files are checked instead of
// running cargo (used by the self-test).
// Exit 0 when clean, 1 on a violation, 2 when the inputs cannot be read.

import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/** Engine crates no non-root first-party package may reach. */
export const FORBIDDEN_DEPS = [
  'rusqlite',
  'libsqlite3-sys',
  'mongodb',
  'bson',
  'aws-sdk-s3',
  'lapin',           // AMQP client (RabbitMQ)
  'amq-protocol',    // AMQP wire protocol
  'object_store',
  'sqlx',
  'redb',
  'sled',
  'rocksdb',
  'fs2',             // advisory file locking
];

/** Patterns matched as a prefix against the normalised crate name. */
export const FORBIDDEN_PREFIXES = ['aws-smithy-'];

/** Engine crates in the tinystoragedrivers family. */
export const FORBIDDEN_TSD_ENGINES = [
  'tinystoragedrivers-sqlite',
  'tinystoragedrivers-mongodb',
  'tinystoragedrivers-file',
  'tinystoragedrivers-s3',
  'tinystoragedrivers-rabbitmq',
  'tinystoragedrivers-document',
  'tinystoragedrivers-kv',
  'tinystoragedrivers-blob',
  'tinystoragedrivers-queue',
  'tinystoragedrivers-secrets',
  'tinystoragedrivers-connect',
];

/** Packages allowed to transitively reach a forbidden engine, with a reason. */
export const ALLOWLIST = {
  // Core storage domain — owns all engine initialisation.
  openhuman:          'core storage domain',
  // Hosts — own keyring access through rusqlite (keyring backend).
  'openhuman-app':    'keyring secrets via rusqlite',
  'openhuman-cli':    'keyring secrets via rusqlite',
  'openhuman-tui':    'keyring secrets via rusqlite',
  'openhuman-rpc':    'keyring secrets via rusqlite',
  // iMessage scanner — reads chat.db directly, read-only.
  tinychannels:       'iMessage chat.db reader: read-only SQLite access',
  // CLI tools that bundle their own engine.
  'tinybox-cli':      'bin target, bundles engine',
  tinyjuice:          'bin target, bundles engine',
};

/**
 * Known engine reaches that are not composition roots, each an exact
 * (package, engine) dependency edge with a reason and its follow-up. Only that
 * edge is excused: any other package or engine still fails, and an entry that
 * no longer matches anything fails as stale (when checking real metadata).
 */
export const ENGINE_EXCEPTIONS = [
  {
    package: 'tinyagents-harness-tools',
    engine: 'fs2',
    reason: 'advisory file locking for the workspace lock file (the `fs` tools)',
    followUp: 'tinyagents: drop fs2 from tinyagents-harness-tools',
  },
  {
    package: 'tinymemory-integrations',
    engine: 'rusqlite',
    reason: 'reads legacy memory SQLite directly',
    followUp: "tinymemory: route tinymemory-integrations' sqlite through the storage owner",
  },
];

const exceptionKey = (pkg, engine) => `${pkg} -> ${engine}`;
const EXCEPTION_KEYS = new Set(ENGINE_EXCEPTIONS.map(e => exceptionKey(e.package, e.engine)));

/** One `stale-exception` violation for every exception not in `used`. */
export function staleExceptions(used) {
  return ENGINE_EXCEPTIONS.filter(e => !used.has(exceptionKey(e.package, e.engine))).map(e => ({
    package: e.package,
    kind: 'stale-exception',
    detail: `ENGINE_EXCEPTIONS entry ${exceptionKey(e.package, e.engine)} no longer matches; remove it`,
  }));
}

export const FORBIDDEN_CRATE_NAME = /-(sqlite|mongodb|file)$/;

export const MANIFESTS = ['Cargo.toml', 'crates/openhuman-app/Cargo.toml'];

export const VENDORED_TSD_DIR = 'vendor/tinystoragedrivers/';
export const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const norm = s => String(s ?? '').replaceAll('\\', '/').replace(/\/+$/, '');

const isOwner = name => name.startsWith('tinystoragedrivers');

const normalisedName = name => name.replace(/_/g, '-');

const isForbiddenDep = name => {
  const n = normalisedName(name);
  if (FORBIDDEN_DEPS.includes(n)) return true;
  if (FORBIDDEN_PREFIXES.some(p => n.startsWith(p))) return true;
  return false;
};

const isForbiddenTsdEngine = name => {
  const n = normalisedName(name);
  return FORBIDDEN_TSD_ENGINES.includes(n) || n === 'tinystoragedrivers';
};

/**
 * First-party = built from a path or a git source, not from a registry.
 * `null` source means path-based (same repo); a git source means git-based.
 * A registry source (starts with "registry+") is third-party.
 */
const isFirstParty = pkg => {
  if (pkg.source === null || pkg.source === undefined) return true;
  return pkg.source.startsWith('git+');
};

export function isUnderVendoredTsd(manifest, root) {
  const prefix = `${norm(root)}/${VENDORED_TSD_DIR}`;
  return norm(manifest).startsWith(prefix);
}

const TSD_GIT = /^git\+https?:\/\/[^?#]*\/tinystoragedrivers(?:\.git)?\?tag=([^#&]+)/;

// ---------------------------------------------------------------------------
// Dependency-closure walk
// ---------------------------------------------------------------------------

/**
 * Walk the resolved normal+build dependency graph and report every
 * first-party non-root package that reaches a forbidden engine. The walk does
 * not pass through a composition root (`roots`): what a root reaches is the
 * root's to own. An edge listed in ENGINE_EXCEPTIONS is not reported; its key
 * is added to `used`.
 *
 * @param {object} metadata  - cargo metadata document
 * @param {string[]} roots   - package names that are composition roots
 * @param {Set<string>} [used] - collects the ENGINE_EXCEPTIONS keys that matched
 * @returns {{ package: string, kind: string, engine: string, path: string[], detail: string }[]}
 */
export function checkEngineClosure(metadata, roots, used = new Set()) {
  const packages = Object.fromEntries(
    (metadata.packages ?? []).map(p => [p.id, p])
  );
  const nodes = Object.fromEntries(
    (metadata.resolve?.nodes ?? []).map(n => [n.id, n])
  );

  // Set of package IDs that are composition roots (always allowed to reach engines)
  const rootIds = new Set(
    (metadata.packages ?? [])
      .filter(p => roots.includes(p.name))
      .map(p => p.id)
  );

  const violations = [];

  for (const pkg of metadata.packages ?? []) {
    if (!isFirstParty(pkg) || isOwner(pkg.name)) continue;
    if (pkg.name in ALLOWLIST) continue;

    const pkg_id = pkg.id;
    if (!nodes[pkg_id]) continue;

    // BFS from this package
    // Each entry: [current_id, current_name, path_names]
    const queue = [[pkg_id, pkg.name, [pkg.name]]];
    const seen = new Set([pkg_id]);

    while (queue.length) {
      const [current_id, current_name, path] = queue.shift();
      const node = nodes[current_id];
      if (!node) continue;

      for (const dep of node.deps ?? []) {
        // Only normal and build edges count. `cargo metadata` reports a normal
        // edge as `kind: null` ("dev", "build", or null); dep_kinds may also be
        // absent, which is a normal edge too.
        const dep_kinds = dep.dep_kinds ?? [{ kind: null }];
        const has_normal_or_build = dep_kinds.some(
          dk => dk.kind == null || dk.kind === 'normal' || dk.kind === 'build'
        );
        if (!has_normal_or_build) continue;

        const dep_id = dep.pkg;
        const dep_pkg = packages[dep_id];
        if (!dep_pkg) continue;

        const dep_name = dep_pkg.name;
        const new_path = [...path, dep_name];

        // A composition root owns what it reaches; do not walk past it.
        if (roots.includes(dep_name)) continue;

        // Forbidden engine found — report it, unless this exact edge is excepted.
        if (isForbiddenDep(dep_name) || isForbiddenTsdEngine(dep_name)) {
          const key = exceptionKey(current_name, dep_name);
          if (EXCEPTION_KEYS.has(key)) {
            used.add(key);
            continue;
          }
          violations.push({
            package: pkg.name,
            kind: 'engine-reach',
            engine: dep_name,
            path: new_path,
            detail: `reaches ${dep_name} via ${path.slice(1).join(' -> ')}`,
          });
          // Don't continue through forbidden crates — path to engine is captured.
          continue;
        }

        // Continue the walk through this dep, regardless of whether it's
        // first-party, owner-owned, or a registry crate.  The BFS always
        // continues so that we can trace a path all the way to a forbidden
        // crate even when it passes through tsd engine crates. `seen` only
        // stops re-walking a package: every edge into an engine is checked
        // above, so an excepted edge never hides another path to it.
        if (seen.has(dep_id)) continue;
        seen.add(dep_id);
        queue.push([dep_id, dep_name, new_path]);
      }
    }
  }

  return violations;
}

// ---------------------------------------------------------------------------
// Rule 2: crate naming
// ---------------------------------------------------------------------------

export function checkCrateNaming(metadata) {
  const out = [];
  for (const pkg of metadata.packages ?? []) {
    if (!isFirstParty(pkg) || isOwner(pkg.name)) continue;
    if (FORBIDDEN_CRATE_NAME.test(pkg.name)) {
      out.push({
        package: pkg.name,
        kind: 'crate-name',
        detail: `crate name matches ${FORBIDDEN_CRATE_NAME}`,
      });
    }
  }
  return out;
}

// ---------------------------------------------------------------------------
// Rules 3 & 4: vendored TSD ownership and tag match
// ---------------------------------------------------------------------------

export function checkVendoredOwner(metadata, root = REPO_ROOT) {
  const out = [];
  const packages = metadata.packages ?? [];
  let vendoredVersion = null;

  // Rule 3: every tsd* package must be a vendored path
  for (const pkg of packages) {
    if (!isOwner(pkg.name)) continue;
    const manifest = norm(pkg.manifest_path);
    if (!pkg.source && isUnderVendoredTsd(manifest, root)) {
      vendoredVersion ??= pkg.version ?? null;
      continue;
    }
    const dependents = [
      ...new Set(
        packages
          .filter(p => p.name !== pkg.name && (p.dependencies ?? []).some(d => d.name === pkg.name))
          .map(p => p.name)
      ),
    ].sort();

    out.push({
      package: pkg.name,
      kind: 'tsd-not-vendored',
      detail:
        `resolves to ${pkg.source ?? manifest} instead of ${norm(root)}/${VENDORED_TSD_DIR}` +
        (dependents.length ? ` (required by ${dependents.join(', ')})` : '') +
        '; bump vendor/tinystoragedrivers or the module\'s tinystoragedrivers tag so they agree',
    });
  }

  // Rule 4: git-tag mismatch
  if (vendoredVersion) {
    const seen = new Set();
    for (const p of packages) {
      if (isOwner(p.name)) continue;
      for (const d of p.dependencies ?? []) {
        if (!isOwner(d.name)) continue;
        // Check the dependency's declared source (what the consumer wrote in Cargo.toml)
        const tag = d.source ? TSD_GIT.exec(d.source)?.[1] : null;
        if (!tag) continue;
        const normalisedTag = tag.replace(/^v/, '');
        if (normalisedTag === vendoredVersion) continue;
        const key = `${p.name}|${tag}`;
        if (seen.has(key)) continue;
        seen.add(key);
        out.push({
          package: p.name,
          kind: 'tsd-tag-mismatch',
          detail:
            `declares tinystoragedrivers tag ${tag} but vendor/tinystoragedrivers is ${vendoredVersion}; ` +
            'bump the submodule or align the tag',
        });
      }
    }
  }

  return out;
}

// ---------------------------------------------------------------------------
// Top-level check
// ---------------------------------------------------------------------------

export function checkMetadata(metadata, root = REPO_ROOT, used = new Set()) {
  const allowlistKeys = Object.keys(ALLOWLIST);

  // Rule 1: engine closure
  const closureViolations = checkEngineClosure(metadata, allowlistKeys, used);

  // Rule 2: crate naming
  const namingViolations = checkCrateNaming(metadata);

  // Rules 3 & 4
  const vendoredViolations = checkVendoredOwner(metadata, root);

  return [...closureViolations, ...namingViolations, ...vendoredViolations];
}

export function formatReport(violations) {
  if (violations.length === 0) {
    return 'storage-owner: ok (no storage backend outside tinystoragedrivers* reached from non-root crates)';
  }
  const lines = violations.map(v => {
    if (v.kind === 'engine-reach') {
      return `  - ${v.package}: reaches ${v.engine} via ${v.path.slice(1, -1).join(' -> ')}`;
    }
    return `  - ${v.package}: ${v.detail}`;
  });
  return [
    'storage-owner: FAILED. No non-root first-party package may reach a storage engine.',
    ...lines,
    '',
    'Allowlist entries (composition roots only):',
    ...Object.entries(ALLOWLIST).map(([k, v]) => `  ${k}: ${v}`),
    '',
    'Add a read-only external-database reader to ALLOWLIST with a reason, or use the tinystoragedrivers ports.',
  ].join('\n');
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

function loadMetadata(root, manifest) {
  const r = spawnSync(
    'cargo',
    ['metadata', '--format-version', '1', '--manifest-path', resolve(root, manifest), '--all-features'],
    { cwd: root, encoding: 'utf8', maxBuffer: 1 << 30 }
  );
  if (r.status !== 0) throw new Error(`cargo metadata failed for ${manifest}: ${r.stderr}`);
  return JSON.parse(r.stdout);
}

export function main(argv) {
  const files = [];
  let root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--metadata') files.push(argv[++i]);
    else root = resolve(argv[i]);
  }

  let docs;
  try {
    docs = files.length
      ? files.map(f => JSON.parse(readFileSync(f, 'utf8')))
      : MANIFESTS.map(m => loadMetadata(root, m));
  } catch (err) {
    console.error(`storage-owner: cannot read inputs: ${err.message}`);
    return 2;
  }

  const seen = new Set();
  const used = new Set();
  const violations = docs
    .flatMap(d => checkMetadata(d, root, used))
    .filter(v => {
      const key = `${v.package}|${v.kind}|${v.detail}`;
      return !seen.has(key) && seen.add(key);
    });
  // An exception must still match the workspace it was written for. Fixture
  // runs (--metadata) check their own inputs, not this workspace.
  if (!files.length) violations.push(...staleExceptions(used));

  console.log(formatReport(violations));
  return violations.length ? 1 : 0;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  process.exit(main(process.argv.slice(2)));
}
