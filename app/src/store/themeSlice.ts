import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

export type ThemeMode = 'light' | 'dark' | 'system';
export type TabBarLabels = 'hover' | 'always';

interface ThemeState {
  mode: ThemeMode;
  tabBarLabels: TabBarLabels;
}

const initialState: ThemeState = { mode: 'system', tabBarLabels: 'hover' };

const themeSlice = createSlice({
  name: 'theme',
  initialState,
  reducers: {
    setThemeMode(state, action: PayloadAction<ThemeMode>) {
      state.mode = action.payload;
    },
    setTabBarLabels(state, action: PayloadAction<TabBarLabels>) {
      state.tabBarLabels = action.payload;
    },
  },
});

export const { setThemeMode, setTabBarLabels } = themeSlice.actions;
export default themeSlice.reducer;

/**
 * Resolves a `ThemeMode` to the concrete `light` or `dark` value that should
 * be applied to `<html>`. `system` consults `prefers-color-scheme`; in non-DOM
 * contexts (SSR, tests without matchMedia) it falls back to light.
 */
export function resolveTheme(mode: ThemeMode): 'light' | 'dark' {
  if (mode !== 'system') return mode;
  try {
    if (typeof window !== 'undefined' && window.matchMedia) {
      return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
    }
  } catch {
    // matchMedia unavailable
  }
  return 'light';
}
