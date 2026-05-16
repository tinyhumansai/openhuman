import { configureStore } from '@reduxjs/toolkit';
import { render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it } from 'vitest';

import localeReducer, { setLocale } from '../../../store/localeSlice';
import { I18nProvider, useT } from '../I18nContext';
import type { Locale } from '../types';

function Probe() {
  const { locale, t } = useT();

  return (
    <>
      <span data-testid="locale">{locale}</span>
      <span>{t('settings.language')}</span>
      <span>{t('clearData.title')}</span>
      <span>{t('bootCheck.quit')}</span>
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
  it('serves Indonesian translations with English fallback for missing keys', () => {
    renderWithLocale('id');

    expect(screen.getByTestId('locale')).toHaveTextContent('id');
    expect(screen.getByText('Bahasa')).toBeInTheDocument();
    expect(screen.getByText('Bersihkan Data Aplikasi')).toBeInTheDocument();
    expect(screen.getByText('Quit')).toBeInTheDocument();
  });
});
