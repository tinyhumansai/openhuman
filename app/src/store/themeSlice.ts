import { createSelector, createSlice, type PayloadAction } from '@reduxjs/toolkit';
import { REHYDRATE } from 'redux-persist';

import {
  familyForThemeId,
  findFamily,
  findPreset,
  PRESET_THEMES,
  resolveFamilyVariant,
  THEME_FAMILIES,
} from '../lib/theme/presets';
import type { FontRole } from '../lib/theme/tokens';
import type { Theme, ThemeFamily } from '../lib/theme/types';

export type ThemeMode = 'light' | 'dark' | 'system';
/** Theme variant preference: explicit light/dark, or follow the OS. */
export type ThemeVariant = 'light' | 'dark' | 'system';

/** Sentinel active-theme id meaning "follow OS light/dark preference". */
export const SYSTEM_THEME_ID = 'system';
/** Default theme family selected on first run. */
export const DEFAULT_FAMILY_ID = 'classic';
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
  /**
   * Hide the live "Agentic task insights" step-by-step timeline in chat
   * (default OFF). When true, the verbose per-agent step rows are collapsed
   * away: the chat shows only the existing message-bubble loading plus a
   * compact blinking "Processing" link while a turn is in flight. The full
   * timeline is still one click away via that link / the "View full agent
   * process Source" affordance, which open the existing side panel.
   */
  hideAgentInsights: boolean;
  /**
   * Active selection: a theme **family** id (`classic`, `ocean`, `matrix`,
   * `hal9000`, `sepia`) or a custom theme id. Combined with
   * {@link ThemeState.themeVariant} to resolve the concrete theme. Legacy values
   * (`light`/`dark`/`system`/`ocean`/`midnight`) from older persisted state are
   * normalized by {@link selectEffectiveTheme}.
   */
  activeThemeId: string;
  /**
   * Which variant of the active family to apply: explicit `light`/`dark` or
   * `system` (follow OS). Mirrors {@link ThemeState.mode} so the simple
   * Appearance toggle and the Theme Studio variant control stay in sync.
   */
  themeVariant: ThemeVariant;
  /** User-authored themes (full or partial token overrides). */
  customThemes: Theme[];
}

const initialState: ThemeState = {
  mode: 'system',
  tabBarLabels: 'hover',
  fontSize: 'medium',
  agentMessageViewMode: 'text',
  developerMode: false,
  hideAgentInsights: false,
  activeThemeId: DEFAULT_FAMILY_ID,
  themeVariant: 'system',
  customThemes: [],
};

const themeSlice = createSlice({
  name: 'theme',
  initialState,
  reducers: {
    setThemeMode(state, action: PayloadAction<ThemeMode>) {
      // The simple Appearance light/dark/system toggle drives the variant of the
      // currently-selected family (it no longer forces the Classic family).
      state.mode = action.payload;
      state.themeVariant = action.payload;
    },
    /** Set the light/dark/system variant of the active family. */
    setThemeVariant(state, action: PayloadAction<ThemeVariant>) {
      state.themeVariant = action.payload;
      state.mode = action.payload;
    },
    /** Select a theme family (or a custom theme id). */
    setActiveFamily(state, action: PayloadAction<string>) {
      state.activeThemeId = action.payload;
    },
    /** Back-compat alias: select any family or custom theme by id. */
    setActiveTheme(state, action: PayloadAction<string>) {
      state.activeThemeId = action.payload;
    },
    /** Insert or replace a custom theme (by id) and make it active. */
    upsertCustomTheme(state, action: PayloadAction<Theme>) {
      const theme = action.payload;
      const idx = state.customThemes.findIndex(t => t.id === theme.id);
      if (idx >= 0) state.customThemes[idx] = theme;
      else state.customThemes.push(theme);
      state.activeThemeId = theme.id;
    },
    /** Remove a custom theme; fall back to the default family if it was active. */
    deleteCustomTheme(state, action: PayloadAction<string>) {
      state.customThemes = state.customThemes.filter(t => t.id !== action.payload);
      if (state.activeThemeId === action.payload) {
        state.activeThemeId = DEFAULT_FAMILY_ID;
      }
    },
    /** Set a single colour token (`"R G B"`); auto-forks a preset to custom. */
    setThemeToken(state, action: PayloadAction<{ key: string; value: string }>) {
      const theme = ensureEditableCustom(state);
      theme.colors[action.payload.key] = action.payload.value;
    },
    /** Set a single font role (CSS stack); auto-forks a preset to custom. */
    setFontRole(state, action: PayloadAction<{ role: FontRole; stack: string }>) {
      const theme = ensureEditableCustom(state);
      theme.fonts[action.payload.role] = action.payload.stack;
    },
    /** Patch the backdrop (mesh/solid/image, dots); auto-forks a preset. */
    setThemeBackdrop(
      state,
      action: PayloadAction<{
        kind?: 'mesh' | 'solid' | 'image';
        imageUrl?: string;
        dots?: boolean;
      }>
    ) {
      const theme = ensureEditableCustom(state);
      const prev = theme.backdrop ?? { kind: 'mesh' as const };
      theme.backdrop = { ...prev, ...action.payload, kind: action.payload.kind ?? prev.kind };
    },
    /**
     * Clear the user's edits on the active custom theme. For a theme forked
     * from a preset, restore that preset's base palette/fonts (via `basedOn`)
     * rather than the generic Light/Dark defaults; otherwise clear to empty.
     */
    resetActiveTheme(state) {
      const theme = state.customThemes.find(t => t.id === state.activeThemeId);
      if (!theme) return;
      const base = theme.basedOn ? findPreset(theme.basedOn) : undefined;
      theme.colors = base ? { ...base.colors } : {};
      theme.fonts = base ? { ...base.fonts } : {};
      theme.gradient = base?.gradient ? { ...base.gradient } : undefined;
      theme.backdrop = base?.backdrop ? { ...base.backdrop } : undefined;
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
    setHideAgentInsights(state, action: PayloadAction<boolean>) {
      state.hideAgentInsights = action.payload;
    },
  },
  extraReducers: builder => {
    builder.addCase(REHYDRATE, (state, action) => {
      const rehydrateAction = action as {
        type: typeof REHYDRATE;
        key?: string;
        payload?: Partial<ThemeState>;
      };
      if (rehydrateAction.key !== 'theme') return;
      const inbound = rehydrateAction.payload;
      if (!inbound || typeof inbound !== 'object') return;
      if (
        !Object.prototype.hasOwnProperty.call(inbound, 'themeVariant') &&
        isThemeMode(inbound.mode)
      ) {
        state.themeVariant = inbound.mode;
        console.debug('[theme] migrated persisted mode to themeVariant', { mode: inbound.mode });
      }
    });
  },
});

export const {
  setThemeMode,
  setThemeVariant,
  setActiveFamily,
  setTabBarLabels,
  setFontSize,
  setAgentMessageViewMode,
  setDeveloperMode,
  setHideAgentInsights,
  setActiveTheme,
  upsertCustomTheme,
  deleteCustomTheme,
  setThemeToken,
  setFontRole,
  setThemeBackdrop,
  resetActiveTheme,
} = themeSlice.actions;
export default themeSlice.reducer;

/** Built-in theme families (static). */
export const selectThemeFamilies = (): ThemeFamily[] => THEME_FAMILIES;

/**
 * All selectable concrete themes: built-in variants followed by user themes.
 * Memoized so the reference is stable while `customThemes` is unchanged.
 */
export const selectAllThemes = createSelector(
  (state: { theme: ThemeState }) => state.theme.customThemes,
  (customThemes): Theme[] => [...PRESET_THEMES, ...(customThemes ?? [])]
);

export const selectActiveThemeId = (state: { theme?: ThemeState }): string =>
  state.theme?.activeThemeId ?? DEFAULT_FAMILY_ID;

export const selectThemeVariant = (state: { theme?: ThemeState }): ThemeVariant =>
  state.theme?.themeVariant ?? state.theme?.mode ?? 'system';

export const selectCustomThemes = (state: { theme?: ThemeState }): Theme[] =>
  state.theme?.customThemes ?? [];

/**
 * Normalize the active selection to a `{ family, variant, custom }` shape,
 * tolerating legacy persisted ids (`light`/`dark`/`system`/`ocean`/`midnight`).
 * Returns `custom` set when a user theme is selected.
 */
function resolveSelection(ts: ThemeState): {
  family?: ThemeFamily;
  variant: ThemeVariant;
  custom?: Theme;
} {
  const sel = ts.activeThemeId ?? DEFAULT_FAMILY_ID;
  const variantPref = ts.themeVariant ?? ts.mode ?? 'system';

  const custom = ts.customThemes?.find(t => t.id === sel);
  if (custom) return { custom, variant: variantPref };

  // Current family-id selection.
  const direct = findFamily(sel);
  if (direct) return { family: direct, variant: variantPref };

  // Legacy concrete-variant / sentinel ids.
  if (sel === SYSTEM_THEME_ID) return { family: findFamily('classic'), variant: 'system' };
  if (sel === 'midnight') return { family: findFamily('ocean'), variant: 'dark' };
  const owner = familyForThemeId(sel);
  if (owner) return { family: owner, variant: owner.dark?.id === sel ? 'dark' : 'light' };
  return { family: findFamily('classic'), variant: variantPref };
}

function isThemeMode(value: unknown): value is ThemeMode {
  return value === 'light' || value === 'dark' || value === 'system';
}

/** The active family id (`''` when a custom theme is selected). */
export function selectActiveFamilyId(state: { theme?: ThemeState }): string {
  if (!state.theme) return DEFAULT_FAMILY_ID;
  const { family, custom } = resolveSelection(state.theme);
  if (custom) return '';
  return family?.id ?? DEFAULT_FAMILY_ID;
}

/**
 * Return the active custom theme for editing. If a preset (or legacy id) is
 * active, transparently fork the current effective theme into a custom theme,
 * make it active, and return it — so editing a preset "just works" and the
 * original preset stays pristine. Idempotent per source theme (reuses
 * `custom-<sourceId>`), so rapid edits don't spawn duplicates.
 */
function ensureEditableCustom(ts: ThemeState): Theme {
  const existingActive = ts.customThemes.find(t => t.id === ts.activeThemeId);
  if (existingActive) return existingActive;

  const base = effectiveThemeFromState(ts);
  const id = `custom-${base.id}`;
  let theme = ts.customThemes.find(t => t.id === id);
  if (!theme) {
    theme = {
      id,
      name: `${base.name} (custom)`,
      isDark: base.isDark,
      builtIn: false,
      basedOn: base.id,
      colors: { ...base.colors },
      fonts: { ...base.fonts },
      gradient: base.gradient ? { ...base.gradient } : undefined,
      backdrop: base.backdrop ? { ...base.backdrop } : undefined,
    };
    ts.customThemes.push(theme);
  }
  ts.activeThemeId = id;
  return theme;
}

/** Resolve a {@link ThemeState} to the concrete {@link Theme} to apply. */
function effectiveThemeFromState(ts: ThemeState): Theme {
  const { family, variant, custom } = resolveSelection(ts);
  const resolved = variant === 'system' ? resolveTheme('system') : variant;
  if (custom) {
    const isDark = resolved === 'dark';
    return custom.isDark === isDark ? custom : { ...custom, isDark };
  }
  const fam = family ?? findFamily('classic')!;
  return resolveFamilyVariant(fam, resolved);
}

/**
 * Resolve the effective {@link Theme} to apply right now. A custom theme is
 * returned directly; otherwise the active family's variant is resolved, with
 * `system` consulting `prefers-color-scheme`.
 */
export function selectEffectiveTheme(state: { theme?: ThemeState }): Theme {
  if (!state.theme) return findFamily('classic')!.light!;
  return effectiveThemeFromState(state.theme);
}

/**
 * Selector for the persisted `hideAgentInsights` preference. Falls back to
 * `false` so existing persisted state (written before this field existed)
 * keeps the verbose timeline visible until the user opts out.
 */
export const selectHideAgentInsights = (state: { theme: ThemeState }): boolean =>
  state.theme.hideAgentInsights ?? false;

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
