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

// Resolve the English map accounting for CJS/ESM interop in test runners
// where `export default` may produce `{ default: { ... } }` instead of the
// raw object. `en` could also be empty if tree-shaken in certain bundlers.
const enMap: Record<string, string> =
  en != null && typeof en === 'object' && 'default' in (en as Record<string, unknown>)
    ? ((en as Record<string, unknown>).default as Record<string, string>)
    : (en as unknown as Record<string, string>);
const enFallback: Record<string, string> =
  enMap && Object.keys(enMap).length > 0 ? enMap : translations.en;

const I18nContext = createContext<I18nContextValue>({
  t: (key: string) => enFallback[key] ?? key,
  locale: 'en',
});

export function I18nProvider({ children }: { children: ReactNode }) {
  const locale = useAppSelector(state => state.locale.current);

  const t = useCallback(
    (key: string): string => {
      const map = translations[locale] ?? enFallback;
      return map[key] ?? enFallback[key] ?? key;
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
