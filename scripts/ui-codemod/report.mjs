#!/usr/bin/env node
/**
 * UI migration work-list generator.
 *
 * Scans `app/src` for hand-rolled markup that a sanctioned `components/ui/`
 * primitive already covers, and emits a bucketed work list. Read-only: it never
 * writes to a source file. `scripts/ui-codemod/rewrite.mjs` is the writer.
 *
 * Buckets are DISJOINT by construction — every file appears in exactly one —
 * because the migration workflow hands one bucket to one agent and they all
 * share a single checkout. Two agents editing one file is the failure mode this
 * partitioning exists to prevent.
 *
 * `hasTest` drives batching, not curiosity: CI gates changed lines at 80% via
 * diff-cover, so an import-only edit to an untested file scores 0% and sinks the
 * batch it rides in. Untested files need a smoke test in the same change.
 *
 *   node scripts/ui-codemod/report.mjs            # human-readable summary
 *   node scripts/ui-codemod/report.mjs --json     # machine-readable, for the workflow
 *   node scripts/ui-codemod/report.mjs --bucket components/settings
 */
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, basename, join, resolve } from 'node:path';

const APP = resolve(import.meta.dirname, '../../app');

/**
 * Each pattern names the primitive that replaces it. `weight` is a rough
 * per-site judgement cost, used only to keep buckets comparably sized — an
 * overlay is a far bigger decision than a `<select>` rename.
 */
const PATTERNS = [
  { id: 'button', rg: '<button', primitive: 'Button', weight: 1 },
  { id: 'input', rg: '<input', primitive: 'TextField / Checkbox', weight: 1 },
  { id: 'select', rg: '<select', primitive: 'NativeSelect', weight: 1 },
  { id: 'textarea', rg: '<textarea', primitive: 'TextArea', weight: 1 },
  { id: 'switch', rg: 'role="switch"', primitive: 'Switch', weight: 2 },
  { id: 'table', rg: '<table', primitive: 'Table', weight: 3 },
  { id: 'overlay', rg: 'fixed inset-0', primitive: 'Dialog / Sheet', weight: 4 },
];

const EXCLUDES = [
  '!*.test.tsx', '!*.test.ts', '!*.spec.ts',
  '!src/components/ui/**',   // the primitives themselves
  '!src/pages/dev/**',       // the gallery demonstrates raw markup on purpose
];

/**
 * Files whose match is a false positive, with the reason. Kept explicit rather
 * than tightened into the regex so a future reader can tell "deliberately out of
 * scope" from "the scanner never saw it".
 */
const DENYLIST = new Map([
  ['src/services/analyticsInteractions.ts',
   'read-only contract: the `[role="switch"]` here is a querySelector string in the delegated analytics listener, not markup'],
]);

/**
 * `fixed inset-0` is how you *style* a Radix overlay as much as how you
 * hand-roll one. A file that already drives its overlay through Radix or our
 * own Dialog/Sheet is migrated, not pending.
 */
const MIGRATED_OVERLAY = /from 'radix-ui'|from '.*\/ui\/(Dialog|Sheet|ModalShell)'|<(Dialog|Sheet|ModalShell)\b/;

function readFile(file) {
  try { return readFileSync(join(APP, file), 'utf8'); } catch { return ''; }
}

function rg(args) {
  try {
    return execFileSync('rg', args, { cwd: APP, encoding: 'utf8' }).trim().split('\n').filter(Boolean);
  } catch (err) {
    if (err.status === 1) return []; // ripgrep: no matches
    throw err;
  }
}

/** A file is "tested" if Vitest's `related` graph would plausibly reach it. */
function hasTest(file) {
  const dir = dirname(file);
  const base = basename(file, '.tsx');
  return (
    existsSync(join(APP, dir, `${base}.test.tsx`)) ||
    existsSync(join(APP, dir, '__tests__', `${base}.test.tsx`))
  );
}

/** Bucket = the two leading path segments, e.g. `components/settings`. */
function bucketOf(file) {
  return file.replace(/^src\//, '').split('/').slice(0, 2).join('/');
}

const files = new Map();
for (const p of PATTERNS) {
  const globArgs = EXCLUDES.flatMap((g) => ['-g', g]);
  for (const line of rg(['--count', '--no-heading', p.rg, 'src', ...globArgs])) {
    const idx = line.lastIndexOf(':');
    const file = line.slice(0, idx);
    const count = Number(line.slice(idx + 1));
    if (DENYLIST.has(file)) continue;
    if (p.id === 'overlay' && MIGRATED_OVERLAY.test(readFile(file))) continue;
    if (!files.has(file)) files.set(file, { file, hasTest: hasTest(file), patterns: {}, weight: 0 });
    const entry = files.get(file);
    entry.patterns[p.id] = count;
    entry.weight += count * p.weight;
  }
}

const buckets = new Map();
for (const entry of files.values()) {
  const name = bucketOf(entry.file);
  if (!buckets.has(name)) buckets.set(name, { name, files: [], weight: 0, untested: 0 });
  const b = buckets.get(name);
  b.files.push(entry);
  b.weight += entry.weight;
  if (!entry.hasTest) b.untested += 1;
}

const ordered = [...buckets.values()].sort((a, b) => b.weight - a.weight);
for (const b of ordered) b.files.sort((x, y) => y.weight - x.weight);

const only = process.argv.includes('--bucket')
  ? process.argv[process.argv.indexOf('--bucket') + 1]
  : null;
const selected = only ? ordered.filter((b) => b.name === only) : ordered;

if (process.argv.includes('--json')) {
  console.log(JSON.stringify({ buckets: selected }, null, 2));
} else {
  const totalSites = [...files.values()].reduce(
    (n, e) => n + Object.values(e.patterns).reduce((a, c) => a + c, 0), 0);
  console.log(`${files.size} files, ${totalSites} sites, ${ordered.length} buckets\n`);
  console.log('bucket                          files  untested  weight  patterns');
  for (const b of selected) {
    const pats = new Set(b.files.flatMap((f) => Object.keys(f.patterns)));
    console.log(
      `${b.name.padEnd(30)} ${String(b.files.length).padStart(5)} ` +
      `${String(b.untested).padStart(9)} ${String(b.weight).padStart(7)}  ${[...pats].join(',')}`
    );
  }
}
