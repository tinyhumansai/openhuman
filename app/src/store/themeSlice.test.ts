import { describe, expect, it } from 'vitest';

import themeReducer, {
  FONT_SIZE_PX,
  type FontSize,
  selectHideAgentInsights,
  setAgentMessageViewMode,
  setFontSize,
  setHideAgentInsights,
  setTabBarLabels,
  setThemeMode,
} from './themeSlice';

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
});
