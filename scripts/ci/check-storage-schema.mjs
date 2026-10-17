#!/usr/bin/env node
// Storage-schema gate (tinyhumansai/openhuman#7992, part of #7973).
//
// Every crate that stores records exports `storage_schema()`; the host composes
// them (`crates/openhuman-core/src/storage/schema.rs`). This check dumps the
// composed schema as JSON (the `storage_schema_dump` example of
// `openhuman-cli`), lints it, and keeps `docs/storage-schema.md` in step.
//
// Lint rules (the spec is docs/specs/schema-v2.md in tinystoragedrivers):
//   1. owner names are `[a-z][a-z0-9_]{1,23}` and `tsd` is reserved;
//   2. a collection or stream is `<owner>.<entity>` with a snake_case entity
//      (lowercase letters, digits and `_`), no version suffix (`_v2`);
//   3. a blob or secret prefix is `<owner>/<path>`;
//   4. a name or prefix is declared by exactly one owner (single ownership);
//   5. indexed fields that hold a time are `*_ms` (no `_at`, `_ns`, `_us`,
//      `_secs`, `_ts` or `_time` suffix);
//   6. a migration step names an item its owner declares (or a legacy source).
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

const OWNER = /^[a-z][a-z0-9_]{1,23}$/;
const ENTITY = /^[a-z][a-z0-9_]*$/;
const VERSION_SUFFIX = /_v\d+$/;
const TIME_SUFFIX = /(_at|_ns|_us|_secs|_ts|_time)$/;
const SCOPES = new Set(['tenant', 'agent', 'module', 'cluster']);

/** Features the dump is built with: the contributor set plus the wallet owner. */
export const DUMP_FEATURES = 'web3';

/** Runs the dump example and returns its JSON. */
export function dumpSchema(root = REPO_ROOT) {
  const run = spawnSync(
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
      DUMP_FEATURES,
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
  const claimed = new Map();
  const owners = new Set();
  for (const { owner, items, steps } of dump.owners) {
    if (!OWNER.test(owner) || owner === 'tsd') {
      problems.push(`owner "${owner}" is not a valid owner name`);
    }
    if (owners.has(owner)) problems.push(`owner "${owner}" is declared twice`);
    owners.add(owner);
    const declared = new Set();
    for (const item of items) {
      declared.add(item.name);
      if (!SCOPES.has(item.scope)) {
        problems.push(`${owner}: "${item.name}" has unknown scope class "${item.scope}"`);
      }
      const where = `${owner}: ${item.kind} "${item.name}"`;
      if (item.kind === 'collection' || item.kind === 'stream') {
        const dot = item.name.indexOf('.');
        const prefix = dot < 0 ? '' : item.name.slice(0, dot);
        const entity = dot < 0 ? '' : item.name.slice(dot + 1);
        if (prefix !== owner || !ENTITY.test(entity)) {
          problems.push(`${where} must be <owner>.<entity> with a snake_case entity`);
        } else if (VERSION_SUFFIX.test(entity)) {
          problems.push(`${where} carries a version suffix; the version lives in the registry`);
        }
      } else if (!item.name.startsWith(`${owner}/`) || item.name === `${owner}/`) {
        problems.push(`${where} must be <owner>/<path>`);
      }
      for (const field of item.index_fields ?? []) {
        if (TIME_SUFFIX.test(field)) {
          problems.push(`${where}: indexed field "${field}" holds a time and must be named *_ms`);
        }
      }
      const key = `${item.kind}:${item.name}`;
      if (claimed.has(key)) {
        problems.push(`${where} is also declared by "${claimed.get(key)}" (single ownership)`);
      } else {
        claimed.set(key, owner);
      }
    }
    for (const step of steps) {
      if (!declared.has(step.item)) {
        problems.push(`${owner}: step "${step.id}" names undeclared item "${step.item}"`);
      }
      if (!(step.to > step.from)) {
        problems.push(`${owner}: step "${step.id}" does not go up (${step.from} -> ${step.to})`);
      }
    }
  }
  return problems;
}

/** Renders docs/storage-schema.md. */
export function render(dump) {
  const out = [
    '# Storage schema',
    '',
    '<!-- Generated by `pnpm docs:generate` (scripts/ci/check-storage-schema.mjs) from the',
    'composed registry (`crates/openhuman-core/src/storage/schema.rs`). Do not edit by hand. -->',
    '',
    'Every crate that stores records declares them in `storage_schema()`; the host composes',
    'the declarations into one registry, prepares the backend from it and migrates every',
    'scope at boot. Names are `<owner>.<entity>`; the scope class says where a record lives',
    '(`tenant`: `local` or `u:<id>`, `agent`: `<tenant>/agent:<id>`, `module`:',
    '`<tenant>/module:<id>`, `cluster`: the shared registry scope).',
    '',
  ];
  const sorted = [...dump.owners].sort((a, b) => a.owner.localeCompare(b.owner));
  for (const { owner, items, steps } of sorted) {
    out.push(`## ${owner}`, '', '| Kind | Name | Version | Scope | Indexed fields |', '| --- | --- | --- | --- | --- |');
    for (const item of [...items].sort((a, b) => a.name.localeCompare(b.name))) {
      out.push(
        `| ${item.kind} | \`${item.name}\` | ${item.version ?? ''} | ${item.scope} | ${(item.index_fields ?? []).map(f => `\`${f}\``).join(', ')} |`
      );
    }
    out.push('');
    if (steps.length > 0) {
      out.push('Migration steps:', '');
      for (const step of steps) {
        const source = step.source ? ` from \`${step.source}\`` : '';
        out.push(`- \`${step.id}\`: \`${step.item}\` v${step.from} to v${step.to}${source}`);
      }
      out.push('');
    }
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
  console.log(`storage schema ok: ${dump.owners.length} owners`);
  return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  process.exit(main(process.argv));
}
