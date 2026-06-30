import { REHYDRATE } from 'redux-persist';
import { describe, expect, it } from 'vitest';

import type { Theme } from '../lib/theme/types';
import themeReducer, {
  deleteCustomTheme,
  FONT_SIZE_PX,
  type FontSize,
  resetActiveTheme,
  selectEffectiveTheme,
  selectHideAgentInsights,
  setActiveTheme,
  setAgentMessageViewMode,
  setFontRole,
  setFontSize,
  setHideAgentInsights,
  setTabBarLabels,
  setThemeMode,
  setThemeToken,
  upsertCustomTheme,
} from './themeSlice';

const customTheme = (overrides: Partial<Theme> = {}): Theme => ({
  id: 'custom-1',
  name: 'My theme',
  isDark: false,
  builtIn: false,
  colors: {},
  fonts: {},
  ...overrides,
});

describe('themeSlice', () => {
  it('defaults fontSize to medium', () => {
    const state = themeReducer(undefined, { type: '@@INIT' });
    expect(state.fontSize).toBe('medium');
  });

  it('defaults assistant message rendering to plain text', () => {
    const state = themeReducer(undefined, { type: '@@INIT' });
    expect(state.agentMessageViewMode).toBe('text');
  });

  it('updates fontSize via setFontSize', () => {
    let state = themeReducer(undefined, { type: '@@INIT' });
    state = themeReducer(state, setFontSize('large'));
    expect(state.fontSize).toBe('large');
    state = themeReducer(state, setFontSize('small'));
    expect(state.fontSize).toBe('small');
  });

  it('leaves mode and tabBarLabels untouched when only fontSize changes', () => {
    let state = themeReducer(undefined, { type: '@@INIT' });
    state = themeReducer(state, setThemeMode('dark'));
    state = themeReducer(state, setTabBarLabels('always'));
    state = themeReducer(state, setFontSize('xlarge'));
    expect(state).toEqual({
      mode: 'dark',
      tabBarLabels: 'always',
      fontSize: 'xlarge',
      agentMessageViewMode: 'text',
      developerMode: false,
      hideAgentInsights: false,
      // setThemeMode('dark') syncs the variant; the active family is unchanged.
      activeThemeId: 'classic',
      themeVariant: 'dark',
      customThemes: [],
    });
  });

  it('updates assistant message view mode', () => {
    let state = themeReducer(undefined, { type: '@@INIT' });
    state = themeReducer(state, setAgentMessageViewMode('text'));
    expect(state.agentMessageViewMode).toBe('text');
  });

  it('defaults hideAgentInsights to false and toggles it', () => {
    let state = themeReducer(undefined, { type: '@@INIT' });
    expect(state.hideAgentInsights).toBe(false);
    expect(selectHideAgentInsights({ theme: state })).toBe(false);

    state = themeReducer(state, setHideAgentInsights(true));
    expect(state.hideAgentInsights).toBe(true);
    expect(selectHideAgentInsights({ theme: state })).toBe(true);
  });

  it('falls back to false when hideAgentInsights is absent from persisted state', () => {
    expect(selectHideAgentInsights({ theme: {} as never })).toBe(false);
  });

  it('maps every font size to a concrete px value', () => {
    const sizes: FontSize[] = ['small', 'medium', 'large', 'xlarge'];
    expect(sizes.map(size => FONT_SIZE_PX[size])).toEqual(['14px', '16px', '18px', '20px']);
  });

  it('keeps medium aligned with the historical 16px root size', () => {
    expect(FONT_SIZE_PX.medium).toBe('16px');
  });

  describe('runtime themes', () => {
    it('defaults to the classic family with the system variant', () => {
      const state = themeReducer(undefined, { type: '@@INIT' });
      expect(state.activeThemeId).toBe('classic');
      expect(state.themeVariant).toBe('system');
      expect(state.customThemes).toEqual([]);
    });

    it('setThemeMode keeps mode and themeVariant in sync (family unchanged)', () => {
      let state = themeReducer(undefined, { type: '@@INIT' });
      state = themeReducer(state, setActiveTheme('ocean'));
      state = themeReducer(state, setThemeMode('light'));
      expect(state.themeVariant).toBe('light');
      expect(state.activeThemeId).toBe('ocean'); // family preserved
      state = themeReducer(state, setThemeMode('system'));
      expect(state.themeVariant).toBe('system');
    });

    it('migrates legacy persisted mode into themeVariant on rehydrate', () => {
      const state = themeReducer(undefined, {
        type: REHYDRATE,
        key: 'theme',
        payload: { mode: 'dark' },
      } as never);
      expect(state.themeVariant).toBe('dark');
      expect(selectEffectiveTheme({ theme: state }).id).toBe('dark');
    });

    it('leaves explicit persisted themeVariant for redux-persist reconciliation', () => {
      const state = themeReducer(undefined, {
        type: REHYDRATE,
        key: 'theme',
        payload: { mode: 'dark', themeVariant: 'light' },
      } as never);
      expect(state.themeVariant).toBe('system');
    });

    it('upserts a custom theme and makes it active', () => {
      let state = themeReducer(undefined, { type: '@@INIT' });
      state = themeReducer(state, upsertCustomTheme(customTheme()));
      expect(state.customThemes).toHaveLength(1);
      expect(state.activeThemeId).toBe('custom-1');
    });

    it('edits colour tokens and font roles only on the active custom theme', () => {
      let state = themeReducer(undefined, { type: '@@INIT' });
      state = themeReducer(state, upsertCustomTheme(customTheme()));
      state = themeReducer(state, setThemeToken({ key: 'surface', value: '10 20 30' }));
      state = themeReducer(state, setFontRole({ role: 'body', stack: 'Comic Sans' }));
      expect(state.customThemes[0].colors.surface).toBe('10 20 30');
      expect(state.customThemes[0].fonts.body).toBe('Comic Sans');
    });

    it('auto-forks a custom theme when editing a built-in preset', () => {
      let state = themeReducer(undefined, { type: '@@INIT' });
      state = themeReducer(state, setActiveTheme('dark'));
      state = themeReducer(state, setThemeToken({ key: 'surface', value: '0 0 0' }));
      expect(state.customThemes).toHaveLength(1);
      expect(state.customThemes[0].id).toBe('custom-dark');
      expect(state.activeThemeId).toBe('custom-dark');
      expect(state.customThemes[0].colors.surface).toBe('0 0 0');
      // A second edit reuses the same fork (no duplicate).
      state = themeReducer(state, setThemeToken({ key: 'content', value: '1 1 1' }));
      expect(state.customThemes).toHaveLength(1);
    });

    it('resets overrides on the active custom theme', () => {
      let state = themeReducer(undefined, { type: '@@INIT' });
      state = themeReducer(state, upsertCustomTheme(customTheme({ colors: { surface: '1 2 3' } })));
      state = themeReducer(state, resetActiveTheme());
      expect(state.customThemes[0].colors).toEqual({});
    });

    it('reset restores the source preset base for a preset fork (not empty)', () => {
      let state = themeReducer(undefined, { type: '@@INIT' });
      // Fork Ocean (light) by editing it, then reset.
      state = themeReducer(state, setActiveTheme('ocean'));
      state = themeReducer(state, setThemeToken({ key: 'surface', value: '0 0 0' }));
      const fork = state.customThemes[0];
      expect(fork.basedOn).toBe('ocean');
      state = themeReducer(state, resetActiveTheme());
      // Ocean's base palette is restored (it defines surface-canvas), not wiped.
      expect(state.customThemes[0].colors['surface-canvas']).toBe('233 242 252');
      expect(state.customThemes[0].colors.surface).not.toBe('0 0 0');
    });

    it('deletes a custom theme and falls back to the default family when active', () => {
      let state = themeReducer(undefined, { type: '@@INIT' });
      state = themeReducer(state, upsertCustomTheme(customTheme()));
      state = themeReducer(state, deleteCustomTheme('custom-1'));
      expect(state.customThemes).toEqual([]);
      expect(state.activeThemeId).toBe('classic');
    });

    it('selectEffectiveTheme resolves a custom theme by id', () => {
      let state = themeReducer(undefined, { type: '@@INIT' });
      state = themeReducer(state, upsertCustomTheme(customTheme({ id: 'custom-x', name: 'X' })));
      expect(selectEffectiveTheme({ theme: state }).id).toBe('custom-x');
    });

    it('applies the selected light/dark variant to custom themes', () => {
      let state = themeReducer(undefined, { type: '@@INIT' });
      state = themeReducer(
        state,
        upsertCustomTheme(
          customTheme({ id: 'custom-x', name: 'X', isDark: false, colors: { surface: '1 2 3' } })
        )
      );

      state = themeReducer(state, setThemeMode('dark'));
      expect(selectEffectiveTheme({ theme: state })).toMatchObject({
        id: 'custom-x',
        isDark: true,
        colors: { surface: '1 2 3' },
      });

      state = themeReducer(state, setThemeMode('light'));
      expect(selectEffectiveTheme({ theme: state }).isDark).toBe(false);
    });

    it('selectEffectiveTheme falls back to a preset for an unknown id', () => {
      let state = themeReducer(undefined, { type: '@@INIT' });
      state = themeReducer(state, setActiveTheme('does-not-exist'));
      // Falls back to the Light preset rather than leaving the UI unthemed.
      expect(selectEffectiveTheme({ theme: state }).id).toBe('light');
    });
  });
});
