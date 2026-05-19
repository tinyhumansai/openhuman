import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

import type { Locale } from '../lib/i18n/types';

// Maps a BCP-47 tag prefix to one of our supported locales. Order matters:
// `in` (legacy Indonesian) must come after `id` (Bahasa) so neither shadows
// the other, and `en` sits last so it loses to a more specific match.
const PREFIX_TO_LOCALE: Array<[string, Locale]> = [
  ['zh', 'zh-CN'],
  ['hi', 'hi'],
  ['es', 'es'],
  ['ar', 'ar'],
  ['fr', 'fr'],
  ['bn', 'bn'],
  ['pt', 'pt'],
  ['ru', 'ru'],
  ['id', 'id'],
  ['in', 'id'],
  ['it', 'it'],
  ['en', 'en'],
];

function detectLocale(): Locale {
  try {
    const normalized = navigator.language?.toLowerCase();
    if (!normalized) return 'en';
    for (const [prefix, locale] of PREFIX_TO_LOCALE) {
      if (normalized.startsWith(prefix)) return locale;
    }
  } catch {
    // browser API unavailable
  }
  return 'en';
}

interface LocaleState {
  current: Locale;
}

const initialState: LocaleState = { current: detectLocale() };

const localeSlice = createSlice({
  name: 'locale',
  initialState,
  reducers: {
    setLocale(state, action: PayloadAction<Locale>) {
      state.current = action.payload;
    },
  },
});

export const { setLocale } = localeSlice.actions;
export default localeSlice.reducer;
