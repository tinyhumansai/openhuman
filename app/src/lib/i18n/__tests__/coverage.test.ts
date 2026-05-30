import { describe, expect, it } from 'vitest';

import enAggregate from '../en';

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
  'pl',
] as const;

interface LocaleModule {
  default: Record<string, string>;
}

/**
 * Eagerly imported locale modules — Vite turns the glob into a static map at
 * build time, so this works in both Vitest and production builds (no dynamic
 * import() at runtime, which CLAUDE.md forbids in app/src code).
 */
const localeModules = import.meta.glob<LocaleModule>('../*.ts', { eager: true });

function loadLocale(locale: string): Record<string, string> {
  const mod = localeModules[`../${locale}.ts`];
  if (!mod) throw new Error(`missing locale file: ${locale}.ts`);
  return mod.default;
}

const enFlat = enAggregate as Record<string, string>;

describe('i18n coverage', () => {
  it.each(LOCALES)('locale %s has a translation file', locale => {
    expect(localeModules[`../${locale}.ts`]).toBeDefined();
  });

  it.each(LOCALES)('locale %s defines every English key', locale => {
    const flat = loadLocale(locale);
    const missing = Object.keys(enFlat).filter(k => !(k in flat));
    expect(missing).toEqual([]);
  });

  it.each(LOCALES)('locale %s defines no keys absent from English', locale => {
    const flat = loadLocale(locale);
    const extra = Object.keys(flat).filter(k => !(k in enFlat));
    expect(extra).toEqual([]);
  });
});
