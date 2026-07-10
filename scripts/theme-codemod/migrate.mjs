#!/usr/bin/env node
/**
 * Theme migration codemod.
 *
 * Collapses audited light/dark Tailwind colour pairings into the canonical
 * semantic utilities (bg-surface, text-content, border-line, …) defined in
 * app/src/styles/tokens.css + tailwind.config.js.
 *
 * Usage (from repo root):
 *   node scripts/theme-codemod/migrate.mjs            # dry-run (default) + report
 *   node scripts/theme-codemod/migrate.mjs --write    # apply changes
 *   node scripts/theme-codemod/migrate.mjs --selftest # fixture assertions, no FS scan
 *
 * Safety:
 *   - Only adjacent `light dark:` pairs (either order, single space) are touched.
 *   - Class boundaries exclude a trailing `/`, so opacity-suffixed utilities
 *     (bg-neutral-900/50) are never matched.
 *   - Test/spec files are skipped so fixtures asserting class strings stay intact.
 *   - Idempotent: re-running over migrated code yields zero changes.
 *   - Default is dry-run; nothing is written without --write.
 */

import { readFileSync, writeFileSync, mkdirSync, readdirSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, relative } from 'node:path';
import { PAIRS, SINGLES } from './map.mjs';

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = join(__dirname, '..', '..');
const SRC_DIR = join(REPO_ROOT, 'app', 'src');
const OUT_DIR = join(REPO_ROOT, 'target', 'theme-codemod');

// Class-boundary fragments: a token must be flanked by string/JSX-className
// delimiters (start/space/quote/backtick/brace/paren/gt) — never a `/`, `-`,
// `:` or word char that would mean it's part of a larger class or opacity suffix.
const LEFT = `(^|[\\s"'\\\`{(>])`;
const RIGHT = `($|[\\s"'\\\`})<])`;

function esc(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/** Build the ordered list of {name, re, to} replacement rules. */
function buildRules() {
  const rules = [];
  for (const [light, dark, to] of PAIRS) {
    const a = esc(light);
    const b = esc(dark);
    // Match either ordering of the adjacent pair.
    rules.push({
      name: `${light} ${dark} → ${to}`,
      re: new RegExp(`${LEFT}(?:${a} ${b}|${b} ${a})${RIGHT}`, 'g'),
      to,
      kind: 'pair',
    });
  }
  for (const [from, to] of SINGLES) {
    rules.push({
      name: `${from} → ${to}`,
      re: new RegExp(`${LEFT}${esc(from)}${RIGHT}`, 'g'),
      to,
      kind: 'single',
    });
  }
  return rules;
}

// A "class run": 2+ adjacent class-like tokens on one line. Used by the grouped
// pass so we can pair a light class with its `dark:` partner even when they
// aren't adjacent (e.g. `border-stone-200 bg-white dark:border-neutral-800
// dark:bg-neutral-900` — all light classes first, then all dark ones).
const CLASS_RUN_RE = /[\w:/.@[\]-]+(?:[ \t]+[\w:/.@[\]-]+)+/g;

/**
 * Grouped-pairing pass: within each class run, if BOTH a mapping's light class
 * and its `dark:` partner are present (in any order, any distance), replace the
 * light class with the semantic class and drop the dark one. Only the prefix-
 * free pairs apply (hover:/focus: states are handled by the adjacent pass).
 */
function transformGrouped(text, pairs, counts) {
  return text.replace(CLASS_RUN_RE, (run) => {
    const toks = run.split(/[ \t]+/);
    const set = new Set(toks);
    let changed = false;
    for (const [light, dark, to] of pairs) {
      if (!set.has(light) || !set.has(dark)) continue;
      for (let i = 0; i < toks.length; i++) {
        if (toks[i] === dark) toks[i] = null;
        else if (toks[i] === light) toks[i] = to;
      }
      set.delete(light);
      set.delete(dark);
      set.add(to);
      changed = true;
      const name = `${light} + ${dark} → ${to}`;
      counts[name] = (counts[name] || 0) + 1;
    }
    // Only rewrite runs we actually changed; rebuild with single spaces.
    return changed ? toks.filter((t) => t !== null).join(' ') : run;
  });
}

/**
 * Apply all rules to `text`; returns { out, counts }.
 *
 * Adjacent rules and the grouped pass are looped until the text stabilises: the
 * grouped pass can leave a `hover:` pair adjacent that only the adjacent rules
 * handle, so a single round isn't a fixed point. Looping makes one invocation
 * fully idempotent (a re-run finds nothing).
 */
// Shade→token maps for converting *standalone* dark: neutral utilities (no light
// partner left after the pair passes). Only shades that equal the token's
// built-in dark value are mapped, so the Light/Dark presets look identical and
// only custom themes change.
const DARK_BG = { 950: 'surface-canvas', 900: 'surface', 800: 'surface-muted' };
const DARK_TEXT = { 50: 'content', 100: 'content', 300: 'content-secondary', 400: 'content-muted', 500: 'content-faint' };
const DARK_BORDER = { 800: 'line', 700: 'line-strong' };
const DARK_PLACEHOLDER = { 400: 'content-muted', 500: 'content-faint' };
const DARK_MAPS = { bg: DARK_BG, text: DARK_TEXT, border: DARK_BORDER, placeholder: DARK_PLACEHOLDER };

const DARK_NEUTRAL_RE =
  /\bdark:((?:hover:|focus:|active:|group-hover:|disabled:)*)(bg|text|border|placeholder)-neutral-(\d{2,3})(\/\d+)?\b/g;

/** Convert leftover standalone `dark:[state:]{util}-neutral-N[/op]` to tokens. */
function transformDarkStandalone(text, counts) {
  return text.replace(DARK_NEUTRAL_RE, (m, states, util, shade, opacity) => {
    const token = DARK_MAPS[util]?.[Number(shade)];
    if (!token) return m; // shade with no exact token equivalent — leave as-is
    const name = `dark:${util}-neutral-${shade} → dark:${util}-${token}`;
    counts[name] = (counts[name] || 0) + 1;
    return `dark:${states}${util}-${token}${opacity ?? ''}`;
  });
}

// Light placeholder colours with no dark partner → faint content token.
const LIGHT_PLACEHOLDER_RE = /\b(placeholder)-(?:stone|neutral)-(400|500)\b/g;
function transformLightPlaceholder(text, counts) {
  return text.replace(LIGHT_PLACEHOLDER_RE, (m, util, shade) => {
    const token = shade === '500' ? 'content-muted' : 'content-faint';
    counts[`${m} → ${util}-${token}`] = (counts[`${m} → ${util}-${token}`] || 0) + 1;
    return `${util}-${token}`;
  });
}

const ACCENT_BG_RE = /(?:^|[\s"'`{(>])(?:hover:|focus:|active:|dark:)*bg-(?:primary|coral|sage|amber)-\d/;

/** Invert `text-white` to `text-content-inverted` when on an accent fill. */
function transformInvertedText(text, counts) {
  return text.replace(CLASS_RUN_RE, (run) => {
    if (!run.includes('text-white')) return run;
    if (!ACCENT_BG_RE.test(' ' + run)) return run;
    return run
      .split(/[ \t]+/)
      .map((tkn) => {
        if (tkn !== 'text-white') return tkn;
        counts['text-white → text-content-inverted'] =
          (counts['text-white → text-content-inverted'] || 0) + 1;
        return 'text-content-inverted';
      })
      .join(' ');
  });
}

// Bare (non-dark:) singles with no dark partner. CRITICAL: only convert where
// the token's built-in LIGHT value matches the source shade — otherwise a dark
// bare surface (e.g. bg-stone-900 tooltip, bg-stone-800 active dot) would invert
// to a near-white token in light mode. So:
//   - text: all shades (content* flips correctly for readability in dark themes)
//   - bg/border/divide: ONLY light shades (50–300); dark shades (700–950) are
//     intentionally-dark, ambiguous, and left alone.
const BARE_TEXT = { 300: 'content-faint', 400: 'content-faint', 500: 'content-muted', 600: 'content-secondary', 700: 'content-secondary', 800: 'content', 900: 'content' };
const BARE_BG = { 50: 'surface-muted', 100: 'surface-subtle', 200: 'surface-strong' };
const BARE_BORDER = { 100: 'line-subtle', 200: 'line', 300: 'line-strong' };
const BARE_DIVIDE = { 100: 'line-subtle', 200: 'line', 300: 'line-strong' };
const BARE_MAPS = { text: BARE_TEXT, bg: BARE_BG, border: BARE_BORDER, divide: BARE_DIVIDE };

const BARE_NEUTRAL_RE =
  /(^|[\s"'`{(>])((?:hover:|focus:|active:|group-hover:|disabled:)*)(text|bg|border|divide)-(?:stone|neutral|gray)-(\d{2,3})(\/\d+)?(?=$|[\s"'`})<])/g;

/** Convert bare `{util}-{stone|neutral|gray}-N[/op]` (no dark: partner) to tokens. */
function transformBareNeutral(text, counts) {
  return text.replace(BARE_NEUTRAL_RE, (m, lead, states, util, shade, opacity) => {
    const token = BARE_MAPS[util]?.[Number(shade)];
    if (!token) return m;
    counts[`${util}-*-${shade} → ${util}-${token}`] =
      (counts[`${util}-*-${shade} → ${util}-${token}`] || 0) + 1;
    return `${lead}${states}${util}-${token}${opacity ?? ''}`;
  });
}

// Bare bg-white (incl. opacity) → surface token. bg-black is left alone (it's
// almost always an intentional fixed scrim/media background).
const BARE_WHITE_RE = /(^|[\s"'`{(>])bg-white(\/\d+)?(?=$|[\s"'`})<])/g;
function transformBareWhite(text, counts) {
  return text.replace(BARE_WHITE_RE, (m, lead, opacity) => {
    counts['bg-white → bg-surface'] = (counts['bg-white → bg-surface'] || 0) + 1;
    return `${lead}bg-surface${opacity ?? ''}`;
  });
}

function transform(text, rules) {
  let out = text;
  const counts = {};
  let prev;
  do {
    prev = out;
    for (const rule of rules) {
      out = out.replace(rule.re, (_m, l, r) => {
        counts[rule.name] = (counts[rule.name] || 0) + 1;
        return `${l}${rule.to}${r}`;
      });
    }
    out = transformGrouped(out, PAIRS, counts);
  } while (out !== prev);
  // Post-passes (run once; each is idempotent on its own output).
  out = transformDarkStandalone(out, counts);
  out = transformLightPlaceholder(out, counts);
  out = transformInvertedText(out, counts);
  out = transformBareNeutral(out, counts);
  out = transformBareWhite(out, counts);
  return { out, counts };
}

function isSkippable(path) {
  return (
    /\.(test|spec)\.[tj]sx?$/.test(path) ||
    /\.d\.ts$/.test(path) ||
    path.includes(`${join('lib', 'i18n')}`) // locale string maps — never touch
  );
}

function walk(dir, acc = []) {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    const st = statSync(full);
    if (st.isDirectory()) {
      if (entry === 'node_modules' || entry === '__snapshots__') continue;
      walk(full, acc);
    } else if (/\.(tsx?|jsx?)$/.test(entry) && !isSkippable(full)) {
      acc.push(full);
    }
  }
  return acc;
}

function runSelfTest() {
  const rules = buildRules();
  const cases = [
    ['<div className="bg-white dark:bg-neutral-900 p-4">', '<div className="bg-surface p-4">'],
    ['className="p-2 dark:bg-neutral-900 bg-white"', 'className="p-2 bg-surface"'], // reversed
    [
      'className="text-stone-500 dark:text-neutral-400"',
      'className="text-content-muted"',
    ],
    [
      'className="border-stone-200 dark:border-neutral-800 rounded"',
      'className="border-line rounded"',
    ],
    // Grouped pattern: light classes first, dark classes after (non-adjacent).
    [
      'className="border p-3 transition-colors border-stone-200 bg-white dark:border-neutral-800 dark:bg-neutral-900"',
      'className="border p-3 transition-colors border-line bg-surface"',
    ],
    [
      'className="bg-stone-100 font-medium text-stone-900 dark:bg-neutral-800 dark:text-neutral-100"',
      'className="bg-surface-subtle font-medium text-content"',
    ],
    [
      'className="relative flex w-full bg-white shadow-xl dark:bg-neutral-900"',
      'className="relative flex w-full bg-surface shadow-xl"',
    ],
    // Grouped hover pairs (all hover-light first, then hover-dark).
    [
      'className="transition-colors text-content-secondary hover:bg-stone-50 hover:text-stone-900 dark:hover:bg-neutral-800/60 dark:hover:text-neutral-100"',
      'className="transition-colors text-content-secondary hover:bg-surface-hover hover:text-content"',
    ],
    // Standalone dark: neutral (no light partner) → themed dark token.
    ['className="dark:bg-neutral-900"', 'className="dark:bg-surface"'],
    ['className="dark:hover:bg-neutral-800/60"', 'className="dark:hover:bg-surface-muted/60"'],
    ['className="dark:text-neutral-100"', 'className="dark:text-content"'],
    // text-white on an accent fill → inverted (plain text-white untouched).
    ['className="bg-primary-500 text-white"', 'className="bg-primary-500 text-content-inverted"'],
    ['className="absolute inset-0 text-white"', 'className="absolute inset-0 text-white"'],
    // Placeholder colours.
    ['className="placeholder-stone-400"', 'className="placeholder-content-faint"'],
    ['className="bg-white text-content dark:bg-neutral-600"', 'className="bg-surface text-content"'],
    [
      'className="hover:bg-stone-50 dark:hover:bg-neutral-800"',
      'className="hover:bg-surface-hover"',
    ],
    ['className="font-display text-xl"', 'className="font-title text-xl"'],
    // Opacity-suffixed must be LEFT ALONE:
    // Bare (non-dark:) opacity colours are never matched.
    ['className="bg-neutral-900/50"', 'className="bg-neutral-900/50"'],
    // Standalone dark: opacity colour is themed (light side stays since no pair).
    ['className="bg-white dark:bg-neutral-900/50"', 'className="bg-surface dark:bg-surface/50"'],
    // Bare singles: text (all shades) + light bg/border/divide + bg-white.
    ['className="text-stone-500 px-2"', 'className="text-content-muted px-2"'],
    ['className="text-neutral-900"', 'className="text-content"'],
    ['className="bg-stone-50 p-4"', 'className="bg-surface-muted p-4"'],
    ['className="bg-white p-4"', 'className="bg-surface p-4"'],
    ['className="border-stone-200 rounded"', 'className="border-line rounded"'],
    ['className="divide-y divide-stone-200"', 'className="divide-y divide-line"'],
    ['className="hover:bg-stone-100 rounded"', 'className="hover:bg-surface-subtle rounded"'],
    // Dark bare surfaces are LEFT ALONE (ambiguous; would invert in light mode).
    ['className="bg-stone-900 p-6"', 'className="bg-stone-900 p-6"'],
    ['className="bg-stone-800"', 'className="bg-stone-800"'],
    ['className="bg-black/50"', 'className="bg-black/50"'],
    // Accent untouched.
    ['className="text-primary-600 bg-sage-50"', 'className="text-primary-600 bg-sage-50"'],
    // Unrelated classes untouched:
    ['className="rounded-lg p-2 shadow"', 'className="rounded-lg p-2 shadow"'],
    // Idempotent (already migrated):
    ['className="bg-surface text-content"', 'className="bg-surface text-content"'],
  ];
  let failed = 0;
  for (const [input, expected] of cases) {
    const { out } = transform(input, rules);
    if (out !== expected) {
      failed++;
      console.error(`FAIL\n  in:  ${input}\n  got: ${out}\n  exp: ${expected}`);
    }
  }
  if (failed) {
    console.error(`\n${failed}/${cases.length} self-test cases failed`);
    process.exit(1);
  }
  console.log(`self-test: all ${cases.length} cases passed`);
}

function main() {
  const args = new Set(process.argv.slice(2));
  if (args.has('--selftest')) return runSelfTest();

  const write = args.has('--write');
  const rules = buildRules();
  const files = walk(SRC_DIR);

  const totals = {};
  const changedFiles = [];
  let totalReplacements = 0;

  for (const file of files) {
    const src = readFileSync(file, 'utf8');
    const { out, counts } = transform(src, rules);
    const fileCount = Object.values(counts).reduce((a, b) => a + b, 0);
    if (fileCount === 0) continue;
    changedFiles.push({ file: relative(REPO_ROOT, file), count: fileCount });
    totalReplacements += fileCount;
    for (const [k, v] of Object.entries(counts)) totals[k] = (totals[k] || 0) + v;
    if (write) writeFileSync(file, out, 'utf8');
  }

  // Report
  mkdirSync(OUT_DIR, { recursive: true });
  const lines = [];
  lines.push(`Theme codemod ${write ? '(WRITE)' : '(DRY-RUN)'} — ${new Date().toISOString?.() ?? ''}`);
  lines.push(`Scanned ${files.length} files`);
  lines.push(`Changed ${changedFiles.length} files, ${totalReplacements} replacements\n`);
  lines.push('Per-rule counts:');
  for (const [k, v] of Object.entries(totals).sort((a, b) => b[1] - a[1])) {
    lines.push(`  ${String(v).padStart(5)}  ${k}`);
  }
  lines.push('\nTop changed files:');
  for (const { file, count } of changedFiles.sort((a, b) => b.count - a.count).slice(0, 40)) {
    lines.push(`  ${String(count).padStart(4)}  ${file}`);
  }
  const report = lines.join('\n') + '\n';
  writeFileSync(join(OUT_DIR, 'report.txt'), report, 'utf8');

  console.log(report);
  console.log(`Report written to ${relative(REPO_ROOT, join(OUT_DIR, 'report.txt'))}`);
  if (!write) console.log('\nDry-run only. Re-run with --write to apply.');
}

main();
