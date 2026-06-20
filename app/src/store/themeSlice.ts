import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

export type ThemeMode = 'light' | 'dark' | 'system';
export type TabBarLabels = 'hover' | 'always';
export type AgentMessageViewMode = 'bubbles' | 'text';
/**
 * Global app font size (issue #3120). Drives the root `<html>` font-size, which
 * scales every rem-based Tailwind text utility — including chat messages and the
 * composer — independently of the OS / system font setting.
 */
export type FontSize = 'small' | 'medium' | 'large' | 'xlarge';

/**
 * Single source of truth mapping each {@link FontSize} to the concrete root
 * `font-size` applied to `<html>`. `medium` (16px) matches the historical
 * `:root` size, so existing users see no change after the field defaults in.
 * Consumed by `ThemeProvider`; keep this the only place the px values live.
 */
export const FONT_SIZE_PX: Record<FontSize, string> = {
  small: '14px',
  medium: '16px',
  large: '18px',
  xlarge: '20px',
};

interface ThemeState {
  mode: ThemeMode;
  tabBarLabels: TabBarLabels;
  fontSize: FontSize;
  agentMessageViewMode: AgentMessageViewMode;
  /**
   * Runtime Developer Mode (default OFF).
   * When true, all developer and diagnostic surfaces become visible.
   * Combines with the build-time `IS_DEV` flag — either one enables the gate.
   * Gating is UI-only: the Rust SecurityPolicy / autonomy tier enforcement
   * is authoritative and is never relaxed by this toggle.
   */
  developerMode: boolean;
}

const initialState: ThemeState = {
  mode: 'system',
  tabBarLabels: 'hover',
  fontSize: 'medium',
  agentMessageViewMode: 'text',
  developerMode: false,
};

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
    setFontSize(state, action: PayloadAction<FontSize>) {
      state.fontSize = action.payload;
    },
    setAgentMessageViewMode(state, action: PayloadAction<AgentMessageViewMode>) {
      state.agentMessageViewMode = action.payload;
    },
    setDeveloperMode(state, action: PayloadAction<boolean>) {
      state.developerMode = action.payload;
    },
  },
});

export const {
  setThemeMode,
  setTabBarLabels,
  setFontSize,
  setAgentMessageViewMode,
  setDeveloperMode,
} = themeSlice.actions;
export default themeSlice.reducer;

/**
 * Selector for the persisted `developerMode` preference.
 * Use {@link useDeveloperMode} in components — it combines this with `IS_DEV`.
 */
export const selectDeveloperMode = (state: { theme: ThemeState }): boolean =>
  state.theme.developerMode;

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
