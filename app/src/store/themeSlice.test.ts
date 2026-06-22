import { describe, expect, it } from 'vitest';

import themeReducer, {
  FONT_SIZE_PX,
  type FontSize,
  setAgentMessageViewMode,
  setFontSize,
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
    });
  });

  it('updates assistant message view mode', () => {
    let state = themeReducer(undefined, { type: '@@INIT' });
    state = themeReducer(state, setAgentMessageViewMode('text'));
    expect(state.agentMessageViewMode).toBe('text');
  });

  it('maps every font size to a concrete px value', () => {
    const sizes: FontSize[] = ['small', 'medium', 'large', 'xlarge'];
    expect(sizes.map(size => FONT_SIZE_PX[size])).toEqual(['14px', '16px', '18px', '20px']);
  });

  it('keeps medium aligned with the historical 16px root size', () => {
    expect(FONT_SIZE_PX.medium).toBe('16px');
  });
});
