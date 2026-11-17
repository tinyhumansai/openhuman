import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it } from 'vitest';

import { I18nProvider, useT } from '../lib/i18n/I18nContext';
import localeReducer, { setLocale } from '../store/localeSlice';
import LanguageSelect from './LanguageSelect';

function LanguageSetting() {
  const { t } = useT();
  return <span>{t('settings.language')}</span>;
}

function renderPicker() {
  const store = configureStore({ reducer: { locale: localeReducer } });
  store.dispatch(setLocale('en'));
  render(
    <Provider store={store}>
      <I18nProvider>
        <LanguageSelect />
        <LanguageSetting />
      </I18nProvider>
    </Provider>
  );
  return store;
}

describe('LanguageSelect', () => {
  it('offers Turkish in its own language and applies it immediately', () => {
    const store = renderPicker();
    expect(screen.getByRole('option', { name: '🇹🇷 Türkçe' })).toHaveValue('tr');

    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'tr' } });

    expect(store.getState().locale.current).toBe('tr');
    expect(screen.getByRole('combobox')).toHaveValue('tr');
    expect(screen.getByText('Dil')).toBeInTheDocument();
    expect(document.documentElement.lang).toBe('tr');
    expect(document.documentElement.dir).toBe('ltr');
  });

  it('switches back to English after Turkish', () => {
    const store = renderPicker();
    const picker = screen.getByRole('combobox');
    fireEvent.change(picker, { target: { value: 'tr' } });
    fireEvent.change(picker, { target: { value: 'en' } });

    expect(store.getState().locale.current).toBe('en');
    expect(picker).toHaveValue('en');
    expect(screen.getByText('Language')).toBeInTheDocument();
    expect(document.documentElement.lang).toBe('en');
  });

  it('offers Japanese in its native script and applies it immediately', () => {
    const store = renderPicker();
    expect(screen.getByRole('option', { name: '🇯🇵 日本語' })).toHaveValue('ja');

    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'ja' } });

    expect(store.getState().locale.current).toBe('ja');
    expect(screen.getByRole('combobox')).toHaveValue('ja');
    expect(screen.getByText('言語')).toBeInTheDocument();
    expect(document.documentElement.lang).toBe('ja');
    expect(document.documentElement.dir).toBe('ltr');
  });

  it('can switch back to English after Japanese', () => {
    const store = renderPicker();
    const picker = screen.getByRole('combobox');
    fireEvent.change(picker, { target: { value: 'ja' } });
    fireEvent.change(picker, { target: { value: 'en' } });

    expect(store.getState().locale.current).toBe('en');
    expect(picker).toHaveValue('en');
    expect(screen.getByText('Language')).toBeInTheDocument();
    expect(document.documentElement.lang).toBe('en');
  });
});
