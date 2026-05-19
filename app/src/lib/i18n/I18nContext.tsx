import { createContext, type ReactNode, useCallback, useContext, useEffect, useMemo } from 'react';

import { useAppSelector } from '../../store/hooks';
import ar from './ar';
import bn from './bn';
import en from './en';
import es from './es';
import fr from './fr';
import hi from './hi';
import id from './id';
import it from './it';
import pt from './pt';
import ru from './ru';
import type { Locale } from './types';
import zhCN from './zh-CN';

interface I18nContextValue {
  t: (key: string) => string;
  locale: Locale;
}

const translations: Record<Locale, Record<string, string>> = {
  en,
  'zh-CN': zhCN,
  hi,
  es,
  ar,
  fr,
  bn,
  pt,
  ru,
  id,
  it,
};

// Locales rendered right-to-left.
const RTL_LOCALES: ReadonlySet<Locale> = new Set<Locale>(['ar']);

// Resolve the effective English map at call time. `en` may be wrapped in
// `{ default: { ... } }` by CJS/ESM interop in test runners, or tree-shaken
// to an empty object. We check at each call to handle lazy module resolution.
function resolveEn(): Record<string, string> {
  const raw: Record<string, unknown> = en as unknown as Record<string, unknown>;
  // CJS interop: `import en from './en'` → `{ default: { key: value } }`
  const unwrapped =
    raw != null && typeof raw === 'object' && 'default' in raw && typeof raw.default === 'object'
      ? (raw.default as Record<string, string>)
      : (raw as unknown as Record<string, string>);
  // If `en` resolved to more keys than `translations.en` (which might be
  // the same reference), prefer the richer one.
  if (Object.keys(unwrapped).length > 0) return unwrapped;
  if (Object.keys(translations.en).length > 0) return translations.en;
  return {};
}

const I18nContext = createContext<I18nContextValue>({
  t: (key: string) => {
    const map = resolveEn();
    return map[key] ?? key;
  },
  locale: 'en',
});

export function I18nProvider({ children }: { children: ReactNode }) {
  const locale = useAppSelector(state => state.locale.current);

  // Mirror locale + direction onto <html> so global CSS, browser hyphenation,
  // form controls, scrollbars, etc. pick up the active language.
  useEffect(() => {
    if (typeof document === 'undefined') return;
    const root = document.documentElement;
    root.lang = locale;
    root.dir = RTL_LOCALES.has(locale) ? 'rtl' : 'ltr';
  }, [locale]);

  const t = useCallback(
    (key: string): string => {
      const map = translations[locale] ?? resolveEn();
      return map[key] ?? resolveEn()[key] ?? key;
    },
    [locale]
  );

  const value = useMemo(() => ({ t, locale }), [t, locale]);

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useT(): I18nContextValue {
  return useContext(I18nContext);
}

export { type Locale } from './types';
