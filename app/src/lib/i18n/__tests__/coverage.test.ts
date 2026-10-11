import { describe, expect, it } from 'vitest';

import { REASONING_EFFORTS } from '../../../features/conversations/aui/ReasoningEffortPicker';
import { NODE_KIND_ICON } from '../../../lib/flows/nodeKindIcons';
import { NODE_GROUP_ORDER } from '../../../lib/flows/nodeKindMeta';
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
  'ja',
  'ko',
  'pl',
  'tr',
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

  it.each(['en', ...LOCALES])('%s translates finite dynamic UI options', locale => {
    const flat = locale === 'en' ? enFlat : loadLocale(locale);
    const dynamicKeys = [
      ...REASONING_EFFORTS.map(effort => `composer.reasoning.${effort}`),
      ...Object.keys(NODE_KIND_ICON).map(kind => `flows.nodeKind.${kind}`),
      ...NODE_GROUP_ORDER.map(group => `flows.palette.group.${group}`),
      ...['none', 'subtle', 'default', 'round'].map(corner => `settings.layout.corners.${corner}`),
      ...['subtle', 'default', 'strong'].map(contrast => `settings.layout.contrast.${contrast}`),
    ];
    const missing = dynamicKeys.filter(key => !(key in flat));
    expect(missing).toEqual([]);
  });
  it('locale tr preserves every interpolation placeholder', () => {
    const turkish = loadLocale('tr');
    const placeholders = (value: string) => (value.match(/\{+[^{}]+\}+/g) ?? []).sort();
    const mismatches = Object.entries(enFlat)
      .filter(
        ([key, value]) =>
          JSON.stringify(placeholders(turkish[key] ?? '')) !== JSON.stringify(placeholders(value))
      )
      .map(([key]) => key);
    expect(mismatches).toEqual([]);
  });

  it('preserves Japanese interpolation placeholders exactly', () => {
    const japanese = loadLocale('ja');
    const placeholders = (value: string) => value.match(/\{+[^{}]+\}+/g)?.sort() ?? [];
    const mismatches = Object.entries(enFlat)
      .filter(
        ([key, value]) =>
          JSON.stringify(placeholders(japanese[key])) !== JSON.stringify(placeholders(value))
      )
      .map(([key]) => key);
    expect(mismatches).toEqual([]);
  });

  it.each(['en', ...LOCALES])('locale %s contains no em dashes', locale => {
    const flat = locale === 'en' ? enFlat : loadLocale(locale);
    const keysWithEmDashes = Object.entries(flat)
      .filter(([, value]) => value.includes('\u2014'))
      .map(([key]) => key);
    expect(keysWithEmDashes).toEqual([]);
  });
});
