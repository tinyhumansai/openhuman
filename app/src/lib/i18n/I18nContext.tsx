import { createContext, type ReactNode, useCallback, useContext, useMemo } from 'react';

import { useAppSelector } from '../../store/hooks';
import en from './en';
import type { Locale } from './types';
import zhCN from './zh-CN';

interface I18nContextValue {
  t: (key: string) => string;
  locale: Locale;
}

const translations: Record<Locale, Record<string, string>> = { en, 'zh-CN': zhCN };

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

  const t = useCallback(
    (key: string): string => {
      const map = translations[locale] ?? resolveEn();
      return map[key] ?? resolveEn()[key] ?? key;
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [locale]
  );

  const value = useMemo(() => ({ t, locale }), [t, locale]);

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useT(): I18nContextValue {
  return useContext(I18nContext);
}

export { type Locale } from './types';
