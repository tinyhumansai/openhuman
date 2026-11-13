import type { Locale } from './types';

/**
 * Map the app's active locale to a BCP-47 tag for `Intl`.
 *
 * The app's locales are language codes; `Intl` accepts those directly except
 * for English, where the bare `en` tag resolves to whatever regional default
 * the runtime picks. Pinning English to `en-US` keeps currency output stable
 * ("$12.50", thousands separated by ","), matching the product contract the
 * dashboard is specified against.
 */
const INTL_LOCALES: Record<Locale, string> = {
  en: 'en-US',
  'zh-CN': 'zh-CN',
  hi: 'hi',
  es: 'es',
  ar: 'ar',
  fr: 'fr',
  bn: 'bn',
  pt: 'pt',
  de: 'de',
  ru: 'ru',
  id: 'id',
  it: 'it',
  ja: 'ja',
  ko: 'ko',
  pl: 'pl',
  tr: 'tr',
};

/** BCP-47 locale tag for the app's active language. */
export function intlLocale(locale: Locale): string {
  return INTL_LOCALES[locale] ?? 'en-US';
}
