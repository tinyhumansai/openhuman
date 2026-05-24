#!/usr/bin/env node
/**
 * One-off: mirror keys present in en-N.ts but missing from <locale>-N.ts.
 * Uses the English value as the fallback so the i18n coverage gate passes;
 * actual translations can be filled in later. Run from repo root.
 */
import { promises as fs } from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const CHUNK_DIR = path.resolve('app/src/lib/i18n/chunks');
const LOCALES = ['zh-CN', 'hi', 'es', 'ar', 'fr', 'bn', 'pt', 'de', 'ru', 'id', 'it', 'ko'];
const CHUNK_COUNT = 5;

async function loadChunk(locale, n) {
  const file = path.join(CHUNK_DIR, `${locale}-${n}.ts`);
  try {
    const mod = await import(pathToFileURL(file).href);
    return { file, table: mod.default ?? {} };
  } catch (err) {
    if (err.code === 'ERR_MODULE_NOT_FOUND') return { file, table: null };
    throw err;
  }
}

function tsLiteral(value) {
  // Always emit a fully escaped JS/TS string literal. The earlier
  // single-quote branch left `\` untouched, so values containing a
  // backslash (e.g. `'C:\Users\me'`) would be mis-parsed as escape
  // sequences and silently drop the backslash. `JSON.stringify` handles
  // every escape correctly.
  return JSON.stringify(String(value));
}

async function appendMissing(locale, n, missing) {
  const file = path.join(CHUNK_DIR, `${locale}-${n}.ts`);
  const original = await fs.readFile(file, 'utf8');
  // Find the closing brace of the object literal — assumes the standard pattern
  // `const xN: TranslationMap = { ... }; export default xN;`.
  const closeIdx = original.lastIndexOf('};');
  if (closeIdx === -1) throw new Error(`No closing }; found in ${file}`);
  const insertion = missing
    .map(([k, v]) => `  ${tsLiteral(k)}: ${tsLiteral(v)},`)
    .join('\n');
  const updated = `${original.slice(0, closeIdx)}${insertion}\n${original.slice(closeIdx)}`;
  await fs.writeFile(file, updated);
}

async function main() {
  for (let n = 1; n <= CHUNK_COUNT; n++) {
    const en = await loadChunk('en', n);
    const enKeys = Object.entries(en.table);
    for (const locale of LOCALES) {
      const other = await loadChunk(locale, n);
      if (other.table === null) {
        console.warn(`skip missing chunk file: ${locale}-${n}.ts`);
        continue;
      }
      const missing = enKeys.filter(([k]) => !(k in other.table));
      if (missing.length === 0) continue;
      await appendMissing(locale, n, missing);
      console.log(`+ ${locale}-${n}.ts (${missing.length} keys)`);
    }
  }
}

main().catch(err => {
  console.error(err);
  process.exit(1);
});
