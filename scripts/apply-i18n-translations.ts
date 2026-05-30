#!/usr/bin/env -S pnpm exec tsx
/**
 * apply-i18n-translations — merge a translations file into a single locale file.
 *
 * Input (one file per locale, default dir tmp/i18n-translations/):
 *   {
 *     "locale": "es",
 *     "translations": { "<key>": "<translated string>", ... }
 *   }
 *
 * Behavior:
 *   - Loads the existing app/src/lib/i18n/<locale>.ts (keeps current values for keys
 *     not present in the input).
 *   - Rewrites <locale>.ts containing every English key in en.ts order. Value
 *     precedence: new translation → existing translation → English fallback.
 *   - Single-quoted JS string literals with safe escaping. Header comment preserved.
 *
 * Usage:
 *   pnpm exec tsx scripts/apply-i18n-translations.ts [--dir tmp/i18n-translations] [--locale es]
 */

import { promises as fs } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const ROOT = path.resolve(path.dirname(__filename), "..");
const I18N_DIR = path.join(ROOT, "app/src/lib/i18n");

const LOCALE_HEADERS: Record<string, string> = {
  "zh-CN": "Simplified Chinese (简体中文)",
  hi: "Hindi (हिन्दी)",
  es: "Spanish (Español)",
  ar: "Arabic (العربية)",
  fr: "French (Français)",
  bn: "Bengali (বাংলা)",
  pt: "Portuguese (Português)",
  de: "German (Deutsch)",
  ru: "Russian (Русский)",
  id: "Indonesian (Bahasa Indonesia)",
  it: "Italian (Italiano)",
  ko: "Korean (한국어)",
  pl: "Polish (Polski)",
};

interface InputFile {
  locale: string;
  translations: Record<string, string>;
}

function jsString(s: string): string {
  // Single-quoted JS string literal. Escape backslash, single quote, and any
  // character that would otherwise break out of the single-line literal:
  // \n, \r, \t, U+2028, U+2029, and the rest of the C0 control range.
  const escaped = s.replace(/[\\'\n\r\t\u0000-\u001f\u2028\u2029]/g, (ch) => {
    switch (ch) {
      case "\\":
        return "\\\\";
      case "'":
        return "\\'";
      case "\n":
        return "\\n";
      case "\r":
        return "\\r";
      case "\t":
        return "\\t";
      default:
        return "\\u" + ch.charCodeAt(0).toString(16).padStart(4, "0");
    }
  });
  return "'" + escaped + "'";
}

async function loadLocale(locale: string): Promise<Record<string, string>> {
  const p = path.join(I18N_DIR, `${locale}.ts`);
  const mod = await import(pathToFileURL(p).href);
  if (!mod.default || typeof mod.default !== "object") {
    throw new Error(`${p}: missing default export`);
  }
  return mod.default as Record<string, string>;
}

async function writeLocale(
  locale: string,
  enKeysInOrder: string[],
  values: Record<string, string>,
): Promise<void> {
  const langLabel = LOCALE_HEADERS[locale] ?? locale;
  const lines: string[] = [];
  lines.push(`import type { TranslationMap } from './types';`);
  lines.push("");
  lines.push(`// ${langLabel} translations. Keys mirror en.ts; missing/`);
  lines.push(
    `// English-identical values fall back to English via I18nContext.resolveEn().`,
  );
  lines.push(`const messages: TranslationMap = {`);
  for (const k of enKeysInOrder) {
    const v = values[k];
    if (v === undefined) continue; // shouldn't happen — English fallback ensures coverage
    lines.push(`  ${jsString(k)}: ${jsString(v)},`);
  }
  lines.push(`};`);
  lines.push("");
  lines.push(`export default messages;`);
  lines.push("");
  const file = path.join(I18N_DIR, `${locale}.ts`);
  await fs.writeFile(file, lines.join("\n"));
}

async function applyLocale(
  input: InputFile,
  enKeys: string[],
  enValues: Record<string, string>,
): Promise<{ updated: number; total: number }> {
  const { locale, translations } = input;
  if (locale === "en") throw new Error("refusing to overwrite English source");
  let updated = 0;
  let total = 0;
  let existing: Record<string, string> = {};
  try {
    existing = await loadLocale(locale);
  } catch {
    existing = {};
  }
  const merged: Record<string, string> = {};
  for (const k of enKeys) {
    total++;
    if (Object.prototype.hasOwnProperty.call(translations, k)) {
      const newVal = translations[k];
      if (newVal !== existing[k]) updated++;
      merged[k] = newVal;
    } else if (Object.prototype.hasOwnProperty.call(existing, k)) {
      merged[k] = existing[k];
    } else {
      merged[k] = enValues[k]; // shouldn't trigger if locale is in-sync
    }
  }
  await writeLocale(locale, enKeys, merged);
  return { updated, total };
}

async function main() {
  let dir = path.join(ROOT, "tmp/i18n-translations");
  let onlyLocale: string | null = null;
  const argv = process.argv.slice(2);
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--dir") dir = path.resolve(argv[++i]);
    else if (argv[i] === "--locale") onlyLocale = argv[++i];
    else if (argv[i] === "-h" || argv[i] === "--help") {
      console.log(
        "Usage: pnpm exec tsx scripts/apply-i18n-translations.ts [--dir <dir>] [--locale <code>]",
      );
      process.exit(0);
    } else {
      console.error(`Unknown arg: ${argv[i]}`);
      process.exit(2);
    }
  }
  const enValues = await loadLocale("en");
  const enKeys = Object.keys(enValues);
  const entries = await fs.readdir(dir);
  for (const f of entries) {
    if (!f.endsWith(".json")) continue;
    const locale = f.replace(/\.json$/, "");
    if (onlyLocale && locale !== onlyLocale) continue;
    const raw = await fs.readFile(path.join(dir, f), "utf8");
    const input = JSON.parse(raw) as InputFile;
    if (input.locale !== locale) {
      console.error(
        `! ${f}: locale mismatch (${input.locale} vs ${locale}) — skipping`,
      );
      continue;
    }
    const res = await applyLocale(input, enKeys, enValues);
    console.log(`${locale}: ${res.updated}/${res.total} entries updated`);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(2);
});
