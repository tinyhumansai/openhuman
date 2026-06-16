import { describe, expect, it } from 'vitest';

import layoutReducer, {
  DEFAULT_PANEL_LAYOUT,
  ensurePanelLayout,
  selectPanelLayout,
  setSidebarVisible,
  setSidebarWidth,
  toggleSidebar,
} from './layoutSlice';
import { resetUserScopedState } from './resetActions';

describe('layoutSlice', () => {
  it('starts with no panels', () => {
    const state = layoutReducer(undefined, { type: '@@INIT' });
    expect(state.panels).toEqual({});
  });

  it('seeds a panel from defaults on first ensure only', () => {
    let state = layoutReducer(undefined, { type: '@@INIT' });
    state = layoutReducer(
      state,
      ensurePanelLayout({ id: 'chat', defaults: { sidebarWidth: 300 } })
    );
    expect(state.panels.chat).toEqual({ sidebarVisible: false, sidebarWidth: 300 });

    // A second ensure must not clobber an existing layout.
    state = layoutReducer(state, setSidebarWidth({ id: 'chat', width: 420 }));
    state = layoutReducer(
      state,
      ensurePanelLayout({ id: 'chat', defaults: { sidebarWidth: 100 } })
    );
    expect(state.panels.chat.sidebarWidth).toBe(420);
  });

  it('toggles and sets visibility for a lazily-created panel', () => {
    let state = layoutReducer(undefined, { type: '@@INIT' });
    state = layoutReducer(state, toggleSidebar({ id: 'chat' }));
    expect(state.panels.chat.sidebarVisible).toBe(true);
    state = layoutReducer(state, toggleSidebar({ id: 'chat' }));
    expect(state.panels.chat.sidebarVisible).toBe(false);

    state = layoutReducer(state, setSidebarVisible({ id: 'chat', visible: true }));
    expect(state.panels.chat.sidebarVisible).toBe(true);
  });

  it('persists width independently per panel id', () => {
    let state = layoutReducer(undefined, { type: '@@INIT' });
    state = layoutReducer(state, setSidebarWidth({ id: 'chat', width: 320 }));
    state = layoutReducer(state, setSidebarWidth({ id: 'files', width: 200 }));
    expect(state.panels.chat.sidebarWidth).toBe(320);
    expect(state.panels.files.sidebarWidth).toBe(200);
  });

  it('clears all panels on user-scoped reset', () => {
    let state = layoutReducer(undefined, { type: '@@INIT' });
    state = layoutReducer(state, setSidebarWidth({ id: 'chat', width: 320 }));
    state = layoutReducer(state, resetUserScopedState());
    expect(state.panels).toEqual({});
  });

  describe('selectPanelLayout', () => {
    it('falls back to defaults for an unseen id', () => {
      const root = { layout: { panels: {} } };
      expect(selectPanelLayout('chat')(root)).toEqual(DEFAULT_PANEL_LAYOUT);
      expect(selectPanelLayout('chat', { sidebarVisible: true })(root)).toEqual({
        ...DEFAULT_PANEL_LAYOUT,
        sidebarVisible: true,
      });
    });

    it('returns persisted geometry when present', () => {
      const root = { layout: { panels: { chat: { sidebarVisible: true, sidebarWidth: 333 } } } };
      expect(selectPanelLayout('chat')(root)).toEqual({ sidebarVisible: true, sidebarWidth: 333 });
    });
  });
});
