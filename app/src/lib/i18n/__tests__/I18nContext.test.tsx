import { configureStore } from '@reduxjs/toolkit';
import { render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it } from 'vitest';

import localeReducer, { setLocale } from '../../../store/localeSlice';
import en from '../en';
import { I18nProvider, useT } from '../I18nContext';
import type { Locale, TranslationMap } from '../types';
import zhCN from '../zh-CN';

function unwrapTranslationMap(map: TranslationMap): TranslationMap {
  const raw = map as unknown as Record<string, unknown>;
  return raw != null &&
    typeof raw === 'object' &&
    'default' in raw &&
    typeof raw.default === 'object'
    ? (raw.default as TranslationMap)
    : map;
}

function Probe() {
  const { locale, t } = useT();

  return (
    <>
      <span data-testid="locale">{locale}</span>
      <span>{t('settings.language')}</span>
      <span>{t('clearData.title')}</span>
      <span>{t('bootCheck.quit')}</span>
      <span data-testid="missing-key">{t('this.key.does.not.exist')}</span>
    </>
  );
}

function renderWithLocale(locale: Locale) {
  const store = configureStore({ reducer: { locale: localeReducer } });
  store.dispatch(setLocale(locale));

  return render(
    <Provider store={store}>
      <I18nProvider>
        <Probe />
      </I18nProvider>
    </Provider>
  );
}

describe('I18nProvider', () => {
  it('serves Indonesian translations and falls back to the raw key for unknown keys', () => {
    renderWithLocale('id');

    expect(screen.getByTestId('locale')).toHaveTextContent('id');
    expect(screen.getByText('Bahasa')).toBeInTheDocument();
    expect(screen.getByText('Bersihkan Data Aplikasi')).toBeInTheDocument();
    expect(screen.getByText('Keluar')).toBeInTheDocument();
    // Unknown keys fall through locale → English → raw key.
    expect(screen.getByTestId('missing-key')).toHaveTextContent('this.key.does.not.exist');
  });

  it('keeps the Simplified Chinese locale complete against English keys', () => {
    const englishKeys = Object.keys(unwrapTranslationMap(en));
    const simplifiedChinese = unwrapTranslationMap(zhCN);
    const missingKeys = englishKeys.filter(key => !(key in simplifiedChinese));

    expect(englishKeys.length).toBeGreaterThan(0);
    expect(missingKeys).toEqual([]);
  });
});
