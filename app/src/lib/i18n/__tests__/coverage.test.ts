import { describe, expect, it } from 'vitest';

import enAggregate from '../en';

const CHUNK_COUNT = 5;
const LOCALES = [
  'zh-CN',
  'hi',
  'es',
  'ar',
  'fr',
  'bn',
  'pt',
  'de',
  'ru',
  'id',
  'it',
  'ko',
] as const;

interface ChunkModule {
  default: Record<string, string>;
}

/**
 * Eagerly imported chunk modules — Vite turns the glob into a static map at
 * build time, so this works in both Vitest and production builds (no dynamic
 * import() at runtime, which CLAUDE.md forbids in app/src code).
 */
const chunkModules = import.meta.glob<ChunkModule>('../chunks/*.ts', { eager: true });

function loadChunks(locale: string): Array<Record<string, string> | null> {
  const out: Array<Record<string, string> | null> = [];
  for (let n = 1; n <= CHUNK_COUNT; n++) {
    const key = `../chunks/${locale}-${n}.ts`;
    const mod = chunkModules[key];
    out.push(mod ? mod.default : null);
  }
  return out;
}

function flatten(chunks: Array<Record<string, string> | null>): Record<string, string> {
  const out: Record<string, string> = {};
  for (const c of chunks) {
    if (!c) continue;
    Object.assign(out, c);
  }
  return out;
}

function keyToChunk(chunks: Array<Record<string, string> | null>): Map<string, number> {
  const out = new Map<string, number>();
  chunks.forEach((c, i) => {
    if (!c) return;
    for (const k of Object.keys(c)) out.set(k, i + 1);
  });
  return out;
}

const enChunks = loadChunks('en');
const enFlat = flatten(enChunks);
const enKeyChunk = keyToChunk(enChunks);

describe('i18n coverage', () => {
  it('English aggregate (en.ts) matches the en-N.ts chunks key-for-key', () => {
    const enTsKeys = new Set(Object.keys(enAggregate as Record<string, string>));
    const enChunkKeys = new Set(Object.keys(enFlat));
    const inAggregateOnly = [...enTsKeys].filter(k => !enChunkKeys.has(k));
    const inChunksOnly = [...enChunkKeys].filter(k => !enTsKeys.has(k));
    expect({ inAggregateOnly, inChunksOnly }).toEqual({ inAggregateOnly: [], inChunksOnly: [] });
  });

  it('has no missing English chunk files', () => {
    const missing = enChunks
      .map((c, i) => (c == null ? i + 1 : null))
      .filter((n): n is number => n !== null);
    expect(missing).toEqual([]);
  });

  it.each(LOCALES)('locale %s has no missing chunk files', locale => {
    const missing = loadChunks(locale)
      .map((c, i) => (c == null ? i + 1 : null))
      .filter((n): n is number => n !== null);
    expect(missing).toEqual([]);
  });

  it.each(LOCALES)('locale %s defines every English key', locale => {
    const flat = flatten(loadChunks(locale));
    const missing = Object.keys(enFlat).filter(k => !(k in flat));
    expect(missing).toEqual([]);
  });

  it.each(LOCALES)('locale %s defines no keys absent from English', locale => {
    const flat = flatten(loadChunks(locale));
    const extra = Object.keys(flat).filter(k => !(k in enFlat));
    expect(extra).toEqual([]);
  });

  it.each(LOCALES)('locale %s places each key in the same chunk as English', locale => {
    const localeKeyChunk = keyToChunk(loadChunks(locale));
    const drift: Array<{ key: string; en: number; locale: number }> = [];
    for (const [k, actual] of localeKeyChunk) {
      const expected = enKeyChunk.get(k);
      if (expected !== undefined && expected !== actual) {
        drift.push({ key: k, en: expected, locale: actual });
      }
    }
    expect(drift).toEqual([]);
  });
});
