import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

import type { Locale } from '../lib/i18n/types';

function detectLocale(): Locale {
  try {
    const nav = navigator.language;
    const normalized = nav?.toLowerCase();
    if (normalized?.startsWith('zh')) return 'zh-CN';
    if (normalized?.startsWith('id') || normalized?.startsWith('in')) return 'id';
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
