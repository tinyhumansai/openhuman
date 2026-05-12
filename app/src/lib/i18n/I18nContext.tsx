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

const I18nContext = createContext<I18nContextValue>({
  t: (key: string) => translations.en[key] ?? key,
  locale: 'en',
});

export function I18nProvider({ children }: { children: ReactNode }) {
  const locale = useAppSelector(state => state.locale.current);

  const t = useCallback(
    (key: string): string => {
      const map = translations[locale] ?? translations.en;
      return map[key] ?? translations.en[key] ?? key;
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
