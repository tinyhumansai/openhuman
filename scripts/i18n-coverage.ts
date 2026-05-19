#!/usr/bin/env -S pnpm exec tsx
/**
 * i18n-coverage — surface missing / incomplete / unused / drifted translation keys.
 *
 * Source of truth:  app/src/lib/i18n/chunks/en-{1..5}.ts (aggregated by app/src/lib/i18n/en.ts)
 * Translations:     app/src/lib/i18n/chunks/<locale>-{1..5}.ts (aggregated by <locale>.ts)
 * Locale list:      app/src/lib/i18n/types.ts (Locale union)
 *
 * Reports, per locale:
 *   - missing chunk files
 *   - missing keys (in en, absent in locale) — overall + per chunk
 *   - extra keys (in locale, absent in en)
 *   - placeholder/untranslated entries (value identical to English)
 *   - per-chunk drift (key belongs to en-N but locale put it in <locale>-M, M≠N)
 *
 * Repo-wide:
 *   - unused keys (defined in en, never referenced via t('…') / t("…") in app/src)
 *
 * Usage:  pnpm exec tsx scripts/i18n-coverage.ts [--json] [--locale es,fr] [--no-unused] [--out <dir>]
 *
 * With --out <dir>, writes one JSON per non-English locale (<dir>/<locale>.json) containing
 * categorized work-lists for translators (missing, extra, drift, untranslated with en value).
 */

import { promises as fs } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const ROOT = path.resolve(path.dirname(__filename), "..");
const I18N_DIR = path.join(ROOT, "app/src/lib/i18n");
const CHUNKS_DIR = path.join(I18N_DIR, "chunks");
const APP_SRC = path.join(ROOT, "app/src");
const CHUNK_COUNT = 5;

const ALL_LOCALES = [
  "en",
  "zh-CN",
  "hi",
  "es",
  "ar",
  "fr",
  "bn",
  "pt",
  "ru",
  "id",
  "it",
] as const;
type Locale = (typeof ALL_LOCALES)[number];

interface CliOptions {
  json: boolean;
  locales: Locale[];
  scanUnused: boolean;
  outDir: string | null;
  strictUnused: boolean;
}

function parseArgs(argv: string[]): CliOptions {
  const opts: CliOptions = {
    json: false,
    locales: [...ALL_LOCALES],
    scanUnused: true,
    outDir: null,
    strictUnused: false,
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--json") opts.json = true;
    else if (a === "--no-unused") opts.scanUnused = false;
    else if (a === "--strict-unused") opts.strictUnused = true;
    else if (a === "--out") {
      const out = argv[++i];
      if (!out || out.startsWith("--")) {
        console.error("--out requires a directory path");
        process.exit(2);
      }
      opts.outDir = out;
    } else if (a === "--locale" || a === "--locales") {
      const raw = argv[++i];
      if (!raw || raw.startsWith("--")) {
        console.error("--locale requires a comma-separated locale list");
        process.exit(2);
      }
      const list = raw
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean) as Locale[];
      if (!list.length) {
        console.error("--locale cannot be empty");
        process.exit(2);
      }
      const bad = list.filter((l) => !ALL_LOCALES.includes(l));
      if (bad.length) {
        console.error(
          `Unknown locales: ${bad.join(", ")}. Known: ${ALL_LOCALES.join(", ")}`,
        );
        process.exit(2);
      }
      opts.locales = list;
    } else if (a === "-h" || a === "--help") {
      console.log(
        "Usage: pnpm exec tsx scripts/i18n-coverage.ts [--json] [--locale es,fr] [--no-unused] [--strict-unused] [--out <dir>]",
      );
      process.exit(0);
    } else {
      console.error(`Unknown arg: ${a}`);
      process.exit(2);
    }
  }
  return opts;
}

async function fileExists(p: string): Promise<boolean> {
  try {
    await fs.access(p);
    return true;
  } catch {
    return false;
  }
}

async function loadChunk(
  locale: Locale,
  n: number,
): Promise<Record<string, string> | null> {
  const p = path.join(CHUNKS_DIR, `${locale}-${n}.ts`);
  if (!(await fileExists(p))) return null;
  const mod = await import(pathToFileURL(p).href);
  const val = mod.default;
  if (!val || typeof val !== "object") {
    throw new Error(`${p}: default export is not a translation map`);
  }
  return val as Record<string, string>;
}

interface LocaleData {
  chunks: Array<Record<string, string> | null>; // index 0..CHUNK_COUNT-1
  flat: Record<string, string>;
  keyToChunk: Map<string, number>; // 1-indexed chunk number
  missingChunks: number[]; // 1-indexed
}

async function loadLocale(locale: Locale): Promise<LocaleData> {
  const chunks: Array<Record<string, string> | null> = [];
  const flat: Record<string, string> = {};
  const keyToChunk = new Map<string, number>();
  const missingChunks: number[] = [];

  for (let n = 1; n <= CHUNK_COUNT; n++) {
    const chunk = await loadChunk(locale, n);
    chunks.push(chunk);
    if (chunk === null) {
      missingChunks.push(n);
      continue;
    }
    for (const [k, v] of Object.entries(chunk)) {
      // Last write wins (matches the runtime spread order in <locale>.ts).
      flat[k] = v;
      keyToChunk.set(k, n);
    }
  }
  return { chunks, flat, keyToChunk, missingChunks };
}

async function walkSourceFiles(dir: string, out: string[]): Promise<void> {
  const entries = await fs.readdir(dir, { withFileTypes: true });
  for (const e of entries) {
    if (
      e.name === "node_modules" ||
      e.name === "__tests__" ||
      e.name === "lib/i18n"
    )
      continue;
    const p = path.join(dir, e.name);
    if (e.isDirectory()) {
      // Skip the i18n directory itself — we don't count the definitions as usages.
      if (p.startsWith(I18N_DIR)) continue;
      await walkSourceFiles(p, out);
    } else if (
      e.isFile() &&
      /\.(ts|tsx)$/.test(e.name) &&
      !/\.test\.tsx?$/.test(e.name)
    ) {
      out.push(p);
    }
  }
}

const T_CALL_RE = /\bt\(\s*(['"`])([^'"`]+?)\1/g;

async function collectUsedKeys(): Promise<Set<string>> {
  const files: string[] = [];
  await walkSourceFiles(APP_SRC, files);
  const used = new Set<string>();
  for (const f of files) {
    const src = await fs.readFile(f, "utf8");
    for (const m of src.matchAll(T_CALL_RE)) {
      used.add(m[2]);
    }
  }
  return used;
}

interface LocaleReport {
  locale: Locale;
  missingChunks: number[];
  totalKeys: number;
  missingKeys: string[];
  extraKeys: string[];
  untranslatedKeys: string[]; // value === english value (excluding intentional brand strings)
  driftedKeys: Array<{
    key: string;
    expectedChunk: number;
    actualChunk: number;
  }>;
  perChunk: Array<{ chunk: number; missing: number; total: number }>;
}

function diffKeys(
  en: LocaleData,
  other: LocaleData,
): { missing: string[]; extra: string[] } {
  const enKeys = new Set(Object.keys(en.flat));
  const otherKeys = new Set(Object.keys(other.flat));
  const missing: string[] = [];
  const extra: string[] = [];
  for (const k of enKeys) if (!otherKeys.has(k)) missing.push(k);
  for (const k of otherKeys) if (!enKeys.has(k)) extra.push(k);
  missing.sort();
  extra.sort();
  return { missing, extra };
}

function findUntranslated(en: LocaleData, other: LocaleData): string[] {
  const out: string[] = [];
  for (const [k, v] of Object.entries(other.flat)) {
    const enV = en.flat[k];
    if (enV === undefined) continue;
    if (v === enV && v.trim() !== "") out.push(k);
  }
  out.sort();
  return out;
}

function findDrift(en: LocaleData, other: LocaleData) {
  const out: Array<{
    key: string;
    expectedChunk: number;
    actualChunk: number;
  }> = [];
  for (const [k, actual] of other.keyToChunk) {
    const expected = en.keyToChunk.get(k);
    if (expected !== undefined && expected !== actual) {
      out.push({ key: k, expectedChunk: expected, actualChunk: actual });
    }
  }
  out.sort((a, b) => a.key.localeCompare(b.key));
  return out;
}

function perChunkMissing(en: LocaleData, other: LocaleData) {
  const out: Array<{ chunk: number; missing: number; total: number }> = [];
  for (let n = 1; n <= CHUNK_COUNT; n++) {
    const enChunk = en.chunks[n - 1] ?? {};
    const otherChunk = other.chunks[n - 1] ?? {};
    const enKeys = Object.keys(enChunk);
    let missing = 0;
    for (const k of enKeys) if (!(k in otherChunk)) missing++;
    out.push({ chunk: n, missing, total: enKeys.length });
  }
  return out;
}

function formatReport(
  reports: LocaleReport[],
  unusedKeys: string[] | null,
): string {
  const lines: string[] = [];
  lines.push("# i18n coverage report");
  lines.push("");
  for (const r of reports) {
    lines.push(`## ${r.locale}  (${r.totalKeys} keys)`);
    if (r.missingChunks.length) {
      lines.push(
        `  ! missing chunk files: ${r.missingChunks.map((n) => `${r.locale}-${n}.ts`).join(", ")}`,
      );
    }
    lines.push(`  missing:        ${r.missingKeys.length}`);
    lines.push(`  extra:          ${r.extraKeys.length}`);
    lines.push(
      `  untranslated:   ${r.untranslatedKeys.length}  (value identical to English)`,
    );
    lines.push(
      `  drifted chunks: ${r.driftedKeys.length}  (key in wrong chunk N)`,
    );
    const pc = r.perChunk
      .map((c) => `${c.chunk}:${c.total - c.missing}/${c.total}`)
      .join("  ");
    lines.push(`  per-chunk:      ${pc}`);
    if (r.missingKeys.length) {
      const preview = r.missingKeys.slice(0, 15).join(", ");
      const more =
        r.missingKeys.length > 15
          ? `, … (+${r.missingKeys.length - 15} more)`
          : "";
      lines.push(`    missing[head]: ${preview}${more}`);
    }
    if (r.extraKeys.length) {
      const preview = r.extraKeys.slice(0, 15).join(", ");
      const more =
        r.extraKeys.length > 15 ? `, … (+${r.extraKeys.length - 15} more)` : "";
      lines.push(`    extra[head]:   ${preview}${more}`);
    }
    if (r.driftedKeys.length) {
      const sample = r.driftedKeys
        .slice(0, 5)
        .map(
          (d) =>
            `${d.key} (en-${d.expectedChunk} → ${r.locale}-${d.actualChunk})`,
        )
        .join("; ");
      lines.push(`    drift[head]:   ${sample}`);
    }
    lines.push("");
  }
  if (unusedKeys) {
    lines.push(`## unused English keys: ${unusedKeys.length}`);
    if (unusedKeys.length) {
      const preview = unusedKeys.slice(0, 30).join(", ");
      const more =
        unusedKeys.length > 30 ? `, … (+${unusedKeys.length - 30} more)` : "";
      lines.push(`  ${preview}${more}`);
    }
    lines.push("");
  }
  return lines.join("\n");
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));

  const en = await loadLocale("en");
  if (en.missingChunks.length) {
    console.error(
      `! English chunks missing: ${en.missingChunks.join(", ")} — cannot continue.`,
    );
    process.exit(2);
  }

  // Guard against drift between en.ts (runtime source of truth) and the en-N.ts chunks.
  // Both must agree key-for-key or downstream tests / translator workflows will diverge.
  const enAggregateModule = (await import(
    pathToFileURL(path.join(I18N_DIR, "en.ts")).href
  )) as { default: Record<string, string> };
  const enTsKeys = new Set(Object.keys(enAggregateModule.default));
  const enChunkKeys = new Set(Object.keys(en.flat));
  const inEnTsNotChunks = [...enTsKeys].filter((k) => !enChunkKeys.has(k));
  const inChunksNotEnTs = [...enChunkKeys].filter((k) => !enTsKeys.has(k));
  if (inEnTsNotChunks.length || inChunksNotEnTs.length) {
    console.error(
      `! Drift between en.ts and en-N.ts chunks:\n` +
        `    in en.ts only: ${inEnTsNotChunks.join(", ") || "(none)"}\n` +
        `    in chunks only: ${inChunksNotEnTs.join(", ") || "(none)"}`,
    );
    process.exit(2);
  }

  const reports: LocaleReport[] = [];
  for (const locale of opts.locales) {
    if (locale === "en") continue;
    const data = await loadLocale(locale);
    const { missing, extra } = diffKeys(en, data);
    reports.push({
      locale,
      missingChunks: data.missingChunks,
      totalKeys: Object.keys(data.flat).length,
      missingKeys: missing,
      extraKeys: extra,
      untranslatedKeys: findUntranslated(en, data),
      driftedKeys: findDrift(en, data),
      perChunk: perChunkMissing(en, data),
    });
  }

  if (opts.outDir) {
    await fs.mkdir(opts.outDir, { recursive: true });
    for (const r of reports) {
      const data = await loadLocale(r.locale);
      const untranslated = r.untranslatedKeys.map((k) => ({
        key: k,
        en: en.flat[k],
        current: data.flat[k],
      }));
      const missing = r.missingKeys.map((k) => ({ key: k, en: en.flat[k] }));
      const extra = r.extraKeys.map((k) => ({ key: k, current: data.flat[k] }));
      const drift = r.driftedKeys;
      const out = {
        locale: r.locale,
        generatedAt: new Date().toISOString(),
        counts: {
          total: r.totalKeys,
          missing: missing.length,
          extra: extra.length,
          drift: drift.length,
          untranslated: untranslated.length,
        },
        missing,
        extra,
        drift,
        untranslated,
      };
      const file = path.join(opts.outDir, `${r.locale}.json`);
      await fs.writeFile(file, JSON.stringify(out, null, 2));
      if (!opts.json) console.error(`  wrote ${path.relative(ROOT, file)}`);
    }
  }

  let unused: string[] | null = null;
  if (opts.scanUnused) {
    const used = await collectUsedKeys();
    unused = Object.keys(en.flat)
      .filter((k) => !used.has(k))
      .sort();
  }

  if (opts.json) {
    console.log(
      JSON.stringify(
        {
          enKeyCount: Object.keys(en.flat).length,
          locales: reports,
          unusedKeys: unused,
        },
        null,
        2,
      ),
    );
  } else {
    console.log(formatReport(reports, unused));
  }

  const localeFailure = reports.some(
    (r) =>
      r.missingChunks.length ||
      r.missingKeys.length ||
      r.extraKeys.length ||
      r.driftedKeys.length,
  );
  const unusedFailure = opts.strictUnused && (unused?.length ?? 0) > 0;
  process.exit(localeFailure || unusedFailure ? 1 : 0);
}

main().catch((err) => {
  console.error(err);
  process.exit(2);
});
