#!/usr/bin/env node
// Storage-schema gate (tinyhumansai/openhuman#7992, part of #7973).
//
// Crates expose v2 `CollectionSpec` values; the host ensures each owner's
// declarations separately (`crates/openhuman-core/src/storage/schema.rs`).
// This check dumps those declarations by crate (the `storage_schema_dump`
// example of `openhuman-cli`), lints them, and keeps `docs/storage-schema.md`
// in step.
//
// Lint rules (the spec is docs/specs/schema-v2.md in tinystoragedrivers):
//   1. collection names are `<owner>.<entity>` with a snake_case entity
//      (lowercase letters, digits and `_`);
//   2. each collection name and index name is unique;
//   3. indexed fields that hold a time are `*_ms` (no `_at`, `_ns`, `_us`,
//      `_secs`, `_ts` or `_time` suffix);
//   4. expiry and search declarations are carried through the dump.
//
// Usage:
//   check-storage-schema.mjs                  lint and fail on doc drift
//   check-storage-schema.mjs --write          lint and rewrite the doc
//   check-storage-schema.mjs --schema <json>  use a dump instead of running cargo
//   check-storage-schema.mjs --doc <path>     the doc to check or write
// Exit 0 when clean, 1 on a violation or drift, 2 when the inputs cannot be read.

import { spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
export const REPO_ROOT = resolve(HERE, '..', '..');
export const DOC_PATH = resolve(REPO_ROOT, 'docs', 'storage-schema.md');

const COLLECTION = /^[a-z][a-z0-9_]*\.[a-z][a-z0-9_]*$/;
const TIME_SUFFIX = /(_at|_ns|_us|_secs|_ts|_time)$/;

// TinyChannels keeps these historical source names in its collection grants
// so the owner-local migrations can copy old rows. They are read-only inputs,
// not v2 collection names; renaming them would orphan persisted data.
const LEGACY_SOURCE_COLLECTIONS = new Set([
  'tinychannels-bus\0channels_delivery_pending',
  'tinychannels-bus\0channels_delivery_failed',
  'tinychannels-bus\0channels_delivery_results',
  'tinychannels-bus\0channel_remote_bindings',
  'tinychannels-bus\0channel_remote_busy',
  'tinychannels-bus\0channel_remote_meta',
  'tinychannels-bus\0whatsapp_device',
  'tinychannels-bus\0whatsapp_identity',
  'tinychannels-bus\0whatsapp_session',
  'tinychannels-bus\0whatsapp_prekey',
  'tinychannels-bus\0whatsapp_signed_prekey',
  'tinychannels-bus\0whatsapp_sender_key',
  'tinychannels-bus\0whatsapp_sync_key',
  'tinychannels-bus\0whatsapp_lid_mapping',
]);

/** Features the dump is built with: the contributor set plus the wallet owner. */
export const DUMP_FEATURES = 'web3';

/**
 * Features for this run. `STORAGE_SCHEMA_FEATURES` lets private CI build the
 * dump with the feature set of an existing plain build so Cargo reuses it
 * instead of compiling the core again: a comma list, or `core-product` for
 * `scripts/ci/product-features.sh --core` (what the plain `openhuman-cli`
 * builds use). The schema is only gated on flows, skills, modules and web3,
 * all of which are in the default set plus web3 and in the product set, so the dump is identical; the wallet owner (web3) must be in any
 * override.
 */
export function dumpFeatures(env = process.env, root = REPO_ROOT) {
  let features = env.STORAGE_SCHEMA_FEATURES?.trim() || DUMP_FEATURES;
  if (features === 'core-product') {
    const run = spawnSync('bash', [resolve(root, 'scripts/ci/product-features.sh'), '--core'], { encoding: 'utf8' });
    if (run.status !== 0) throw new Error(`product-features.sh --core failed:\n${run.stderr}`);
    features = run.stdout.trim();
  }
  if (!features.split(',').includes('web3')) {
    throw new Error(`STORAGE_SCHEMA_FEATURES must include web3 (the wallet owner): ${features}`);
  }
  return features;
}

/** Runs the dump example and returns its JSON. */
export function dumpSchema(root = REPO_ROOT, { run: spawn = spawnSync, env = process.env } = {}) {
  const run = spawn(
    'cargo',
    [
      'run',
      '--quiet',
      '--manifest-path',
      resolve(root, 'Cargo.toml'),
      '-p',
      'openhuman-cli',
      '--example',
      'storage_schema_dump',
      '--features',
      dumpFeatures(env, root),
    ],
    { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 }
  );
  if (run.status !== 0) {
    throw new Error(`storage_schema_dump failed:\n${run.stderr}`);
  }
  return JSON.parse(run.stdout);
}

/** @returns {string[]} one message per violation */
export function lint(dump) {
  const problems = [];
  if (!Array.isArray(dump.owners)) {
    return ['schema dump must group collections under an owners array'];
  }
  const crates = new Set();
  const names = new Set();
  for (const owner of dump.owners ?? []) {
    if (!owner.crate || crates.has(owner.crate)) {
      problems.push(`crate name "${owner.crate}" is empty or declared more than once`);
    }
    crates.add(owner.crate);
    for (const collection of owner.collections ?? []) {
      const { name, indexes = [], ttl_field: ttlField, search } = collection;
      if (!COLLECTION.test(name) && !LEGACY_SOURCE_COLLECTIONS.has(`${owner.crate}\0${name}`)) {
        problems.push(`${owner.crate}: collection "${name}" must be <owner>.<entity> with a snake_case entity`);
      }
      const key = `${owner.crate}\0${name}`;
      if (names.has(key)) problems.push(`${owner.crate}: collection "${name}" is declared more than once`);
      names.add(key);
      const indexNames = new Set();
      for (const index of indexes) {
        if (!index.name || indexNames.has(index.name)) {
          problems.push(`${owner.crate}: ${name}: index name "${index.name}" is empty or duplicated`);
        }
        indexNames.add(index.name);
        for (const field of index.fields ?? []) {
          if (TIME_SUFFIX.test(field)) {
            problems.push(`${owner.crate}: ${name}: indexed field "${field}" holds a time and must be named *_ms`);
          }
        }
      }
      if (ttlField && TIME_SUFFIX.test(ttlField)) {
        problems.push(`${owner.crate}: ${name}: expiry field "${ttlField}" holds a time and must be named *_ms`);
      }
      for (const field of search?.fields ?? []) {
        if (!field) problems.push(`${owner.crate}: ${name}: search fields must not be empty`);
      }
    }
  }
  return problems;
}

const compare = (left, right) => (left < right ? -1 : left > right ? 1 : 0);

/** Renders docs/storage-schema.md from stable crate-grouped CollectionSpecs. */
export function render(dump) {
  const out = [
    '# Storage schema',
    '',
    '<!-- Generated by `pnpm docs:generate` (scripts/ci/check-storage-schema.mjs) from the',
    'per-crate `CollectionSpec` values (`crates/openhuman-core/src/storage/schema.rs`). Do not edit by hand. -->',
    '',
    'Each crate declares its collections with `CollectionSpec`; the host calls',
    '`ensure_collection` for every declaration at boot. Sections preserve the declaring',
    'crate, including when two crates use the same collection name.',
    'Record shapes and migrations stay with their owners.',
    '',
  ];
  for (const owner of [...(dump.owners ?? [])].sort((a, b) => compare(a.crate, b.crate))) {
    out.push(`## ${owner.crate}`, '', '| Collection | Indexes | Expiry field | Search fields |', '| --- | --- | --- | --- |');
    for (const collection of [...(owner.collections ?? [])].sort((a, b) => compare(a.name, b.name))) {
      const indexes = (collection.indexes ?? []).map(index =>
        `\`${index.name}\` (${index.fields.map(field => `\`${field}\``).join(', ')})${index.unique ? ' unique' : ''}`
      ).sort(compare).join('<br>');
      out.push(
        `| \`${collection.name}\` | ${indexes} | ${collection.ttl_field ? `\`${collection.ttl_field}\`` : ''} | ${(collection.search?.fields ?? []).map(field => `\`${field}\``).join(', ')} |`
      );
    }
    out.push('');
  }
  return `${out.join('\n').trimEnd()}\n`;
}

function main(argv) {
  const args = argv.slice(2);
  const take = flag => {
    const at = args.indexOf(flag);
    return at < 0 ? undefined : args[at + 1];
  };
  const write = args.includes('--write');
  const docPath = take('--doc') ?? DOC_PATH;
  let dump;
  try {
    const file = take('--schema');
    dump = file ? JSON.parse(readFileSync(file, 'utf8')) : dumpSchema();
  } catch (error) {
    console.error(`check-storage-schema: ${error.message}`);
    return 2;
  }
  const problems = lint(dump);
  if (problems.length > 0) {
    console.error('Storage schema violations:');
    for (const problem of problems) console.error(`  - ${problem}`);
    return 1;
  }
  const rendered = render(dump);
  if (write) {
    writeFileSync(docPath, rendered);
    console.log(`wrote ${docPath}`);
    return 0;
  }
  let current = '';
  try {
    current = readFileSync(docPath, 'utf8');
  } catch {
    /* missing doc counts as drift */
  }
  if (current !== rendered) {
    console.error(`${docPath} is out of date. Run \`pnpm docs:generate\` and commit the result.`);
    return 1;
  }
  const collectionCount = (dump.owners ?? []).reduce((total, owner) => total + (owner.collections ?? []).length, 0);
  console.log(`storage schema ok: ${(dump.owners ?? []).length} crates, ${collectionCount} collections`);
  return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  process.exit(main(process.argv));
}
