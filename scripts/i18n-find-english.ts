#!/usr/bin/env -S pnpm exec tsx
/**
 * i18n-find-english — find locale values that are still (or have drifted back to) English.
 *
 * The coverage gate (i18n-coverage.ts) only flags values byte-identical to the current
 * English string. It cannot see values that were translated from an OLD English string
 * and never re-translated when the English copy changed ("stale English"), nor English
 * prose that simply differs from the current en value. This tool detects both.
 *
 * Detection strategy (per locale):
 *   - Technical literals are skipped (pure placeholders, URLs, single-token identifiers,
 *     file paths, commands, values with no real word).
 *   - Non-Latin-script locales (zh-CN, hi, bn, ar, ru, ko): a non-technical value that
 *     contains NO character of the locale's native script is treated as English.
 *     (High recall — vocabulary-independent.)
 *   - Latin-script locales (de, es, fr, it, pt, id, pl): a non-technical value is flagged
 *     when it is identical to the current English value, OR when it contains >= 2 distinct
 *     English-only function words (the/and/while/may/your/…) that do not exist in any of
 *     these languages. (A vocabulary-ratio test is unreliable here because French/Spanish/
 *     Italian/Portuguese share huge cognate vocabulary with English.)
 *
 * Usage:
 *   pnpm exec tsx scripts/i18n-find-english.ts                 # human report
 *   pnpm exec tsx scripts/i18n-find-english.ts --json          # machine summary
 *   pnpm exec tsx scripts/i18n-find-english.ts --out <dir>     # per-locale work-lists {locale, items:[{key,en}]}
 *   pnpm exec tsx scripts/i18n-find-english.ts --locale de,fr  # subset
 */

import { promises as fs } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const ROOT = path.resolve(path.dirname(__filename), "..");
const I18N_DIR = path.join(ROOT, "app/src/lib/i18n");

const NATIVE_SCRIPT: Record<string, RegExp> = {
  "zh-CN": /[㐀-䶿一-鿿豈-﫿]/,
  hi: /[ऀ-ॿ]/,
  bn: /[ঀ-৿]/,
  ar: /[؀-ۿݐ-ݿࢠ-ࣿﭐ-﷿ﹰ-﻿]/,
  ru: /[Ѐ-ӿ]/,
  ko: /[가-힯ᄀ-ᇿ㄰-㆏]/,
};

const LATIN_LOCALES = ["es", "fr", "pt", "de", "id", "it", "pl"] as const;
const ALL_LOCALES = [...Object.keys(NATIVE_SCRIPT), ...LATIN_LOCALES];

// Keys whose values are intentionally English in every locale: brand/product names,
// shell commands, file paths, glob patterns, code identifiers, example data, unit/technical
// tokens, pure placeholder patterns, and short labels that are valid cognates in the Latin
// locales. These are reviewed exceptions — a value flagged here is expected, not a bug.
// A key NOT in this set that the detector flags is a genuine untranslated string to fix.
const INTENTIONAL_ENGLISH = new Set([
  "app.connectionIndicator.coreOffline",
  "channels.activeRouteValue",
  "composio.integrationSlugsExample",
  "composio.integrationSlugsPlaceholder",
  "devOptions.toolPolicyDiagnostics.mcpAllowlists.allowDeny",
  "intelligence.diagram.skillInstallCommand",
  "intelligence.memoryChunk.detail.embeddingInfo",
  "mcp.playground.argsLabel",
  "memorySources.globPatternPlaceholder",
  "memorySources.searchQueryPlaceholder",
  "migration.vendor.hermes",
  "screenAwareness.debug.defaultPanicHotkey",
  "settings.ai.connectionsPerTick",
  "settings.ai.localModelResolved",
  "settings.ai.localOllama",
  "settings.ai.minutesShort",
  "settings.ai.openAiUrlLabel",
  "settings.billing.inferenceBudget.dailySpendPoint",
  "settings.localModel.download.embeddingModel",
  "settings.localModel.download.ttsOutput",
  "settings.localModel.status.contextOkBadge",
  "settings.localModel.status.expectedChat",
  "settings.localModel.status.expectedVision",
  "settings.mcpServer.clientClaudeDesktop",
  "settings.search.allowedSitesPlaceholder",
  "settings.search.engineBraveLabel",
  "settings.taskSources.name",
  "skills.create.allowedToolsPlaceholder",
  "skills.create.optional",
  "skills.meetingBots.platforms.gmeet",
  "skills.meetingBots.platforms.teams",
  "subconscious.interval.fifteenMinutes",
  "subconscious.interval.fiveMinutes",
  "subconscious.interval.tenMinutes",
  "subconscious.interval.thirtyMinutes",
  "vault.excludesPlaceholder",
  "vault.syncSummaryDuration",
  "voice.providers.chip.piper",
  "voice.providers.chip.whisper",
  "voice.providers.whisperModelBase",
  "walkthrough.tooltip.stepCounter",
  "workspace.obsidianConfigDirPlaceholder",
]);

// Distinctly-English function words that do NOT occur in es/fr/pt/de/id/it/pl. A Latin-script
// value carrying >= 2 of these is almost certainly English. Deliberately excludes ambiguous
// short words shared with those languages (a, in, is, no, to, or, of, on, as, by, an, so…).
const ENGLISH_FN = new Set(
  (
    "the and you your this that these those with for will would shall should can cannot could " +
    "may might must are were was have has had not they them their when which while from than " +
    "then about after before without within into onto upon what who why how here there also " +
    "only just very more most some any each both such please every between during through " +
    "because however therefore otherwise whether doesn isn aren don won enabled disabled"
  ).split(" "),
);

interface CliOptions {
  json: boolean;
  outDir: string | null;
  locales: string[];
}

function parseArgs(argv: string[]): CliOptions {
  const opts: CliOptions = {
    json: false,
    outDir: null,
    locales: [...ALL_LOCALES],
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--json") opts.json = true;
    else if (a === "--out") {
      const v = argv[++i];
      if (!v || v.startsWith("--")) {
        console.error("--out requires a directory path");
        process.exit(2);
      }
      opts.outDir = v;
    } else if (a === "--locale" || a === "--locales") {
      const raw = argv[++i];
      if (!raw) {
        console.error("--locale requires a comma-separated list");
        process.exit(2);
      }
      opts.locales = raw
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean);
      const bad = opts.locales.filter((l) => !ALL_LOCALES.includes(l));
      if (bad.length) {
        console.error(`Unknown locales: ${bad.join(", ")}`);
        process.exit(2);
      }
    } else if (a === "-h" || a === "--help") {
      console.log(
        "Usage: pnpm exec tsx scripts/i18n-find-english.ts [--json] [--out <dir>] [--locale de,fr]",
      );
      process.exit(0);
    } else {
      console.error(`Unknown arg: ${a}`);
      process.exit(2);
    }
  }
  return opts;
}

async function loadLocale(locale: string): Promise<Record<string, string>> {
  const p = path.join(I18N_DIR, `${locale}.ts`);
  const mod = await import(pathToFileURL(p).href);
  return mod.default as Record<string, string>;
}

/** Strip placeholders {…}, URLs, and bracketed/parenthetical literals, then return lowercase words. */
function contentWords(value: string): string[] {
  const stripped = value
    .replace(/\{[^}]*\}/g, " ") // placeholders
    .replace(/https?:\/\/\S+/g, " ") // URLs
    .replace(/[A-Z][A-Z0-9_]{3,}/g, " "); // SCREAMING_SNAKE constants
  // Unicode-aware tokenization so accented words stay whole (e.g. "connecté" must not
  // truncate to the English-looking stem "connect").
  return (stripped.toLowerCase().match(/\p{L}[\p{L}']*/gu) ?? []).filter(
    (w) => w.length >= 2,
  );
}

function isTechnical(value: string): boolean {
  const s = value.trim();
  if (s === "") return true;
  if (!/[A-Za-z]{2,}/.test(s)) return true; // only symbols/numbers/placeholders
  if (/^\{[^}]*\}[%s]?$/.test(s)) return true; // pure placeholder
  if (/^https?:\/\//.test(s)) return true;
  // single token: identifier / path / command-ish / model id
  if (!/\s/.test(s) && /^[A-Za-z0-9._:/@+%·✓•…#—–{}'-]+$/.test(s)) return true;
  return false;
}

function looksEnglish(value: string): boolean {
  const distinct = new Set(
    contentWords(value).filter((w) => ENGLISH_FN.has(w)),
  );
  return distinct.size >= 2;
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const en = await loadLocale("en");

  const perLocale: Record<
    string,
    Array<{ key: string; en: string; current: string }>
  > = {};
  for (const locale of opts.locales) {
    const map = await loadLocale(locale);
    const native = NATIVE_SCRIPT[locale];
    const items: Array<{ key: string; en: string; current: string }> = [];
    for (const [k, v] of Object.entries(map)) {
      if (isTechnical(v)) continue;
      if (INTENTIONAL_ENGLISH.has(k)) continue;
      const flagged = native
        ? !native.test(v) // non-Latin: no native char ⇒ English
        : v === en[k] || looksEnglish(v); // Latin: identical or >=2 English-only function words
      if (flagged) items.push({ key: k, en: en[k], current: v });
    }
    items.sort((a, b) => a.key.localeCompare(b.key));
    perLocale[locale] = items;
  }

  if (opts.outDir) {
    await fs.mkdir(opts.outDir, { recursive: true });
    for (const [locale, items] of Object.entries(perLocale)) {
      await fs.writeFile(
        path.join(opts.outDir, `${locale}.json`),
        JSON.stringify({ locale, count: items.length, items }, null, 2),
      );
    }
  }

  const counts = Object.fromEntries(
    Object.entries(perLocale).map(([l, i]) => [l, i.length]),
  );
  const total = Object.values(counts).reduce((a, b) => a + b, 0);

  if (opts.json) {
    console.log(JSON.stringify({ counts, total }, null, 2));
  } else {
    console.log("# i18n English-leftover report\n");
    for (const [l, items] of Object.entries(perLocale)) {
      console.log(`  ${l.padEnd(6)} ${items.length}`);
      for (const it of items.slice(0, 20)) {
        console.log(
          `      ${it.key}  ${JSON.stringify(it.current).slice(0, 60)}`,
        );
      }
    }
    console.log(`\n  total unexpected English: ${total}`);
    if (total === 0) {
      console.log(
        "  ✓ no unexpected untranslated English (intentional literals allowlisted)",
      );
    }
  }
  // Non-zero ⇒ a non-allowlisted value is still English: fail so this can gate CI.
  process.exit(total > 0 ? 1 : 0);
}

main().catch((err) => {
  console.error(err);
  process.exit(2);
});
