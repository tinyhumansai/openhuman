import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

import { resetUserScopedState } from './resetActions';

/**
 * Persisted geometry for a single two-pane layout (see `TwoPanelLayout`).
 * Keyed by an opaque panel `id` so multiple independent two-pane screens
 * (chat today, others later) each remember their own sidebar state without
 * colliding.
 */
export interface PanelLayout {
  /** Whether the mini sidebar is shown. */
  sidebarVisible: boolean;
  /** Sidebar width in CSS px, as set by dragging the divider. */
  sidebarWidth: number;
}

interface LayoutState {
  panels: Record<string, PanelLayout>;
}

const initialState: LayoutState = { panels: {} };

/**
 * Default geometry applied the first time a panel id is seen. Component-level
 * defaults (passed as props) win on initial mount; this is only the fallback
 * baked into the slice so reducers can operate without the component handy.
 */
export const DEFAULT_PANEL_LAYOUT: PanelLayout = {
  sidebarVisible: false,
  sidebarWidth: 256, // matches the legacy `w-64` thread sidebar
};

function panelFor(state: LayoutState, id: string): PanelLayout {
  if (!state.panels[id]) {
    state.panels[id] = { ...DEFAULT_PANEL_LAYOUT };
  }
  return state.panels[id];
}

const layoutSlice = createSlice({
  name: 'layout',
  initialState,
  reducers: {
    /**
     * Seed a panel's geometry from the component's own defaults on first
     * mount. No-op if the id already has persisted state, so a reload keeps
     * the user's last layout rather than snapping back to the prop defaults.
     */
    ensurePanelLayout: (
      state,
      action: PayloadAction<{ id: string; defaults: Partial<PanelLayout> }>
    ) => {
      const { id, defaults } = action.payload;
      if (!state.panels[id]) {
        state.panels[id] = { ...DEFAULT_PANEL_LAYOUT, ...defaults };
      }
    },
    setSidebarVisible: (state, action: PayloadAction<{ id: string; visible: boolean }>) => {
      panelFor(state, action.payload.id).sidebarVisible = action.payload.visible;
    },
    toggleSidebar: (state, action: PayloadAction<{ id: string }>) => {
      const panel = panelFor(state, action.payload.id);
      panel.sidebarVisible = !panel.sidebarVisible;
    },
    setSidebarWidth: (state, action: PayloadAction<{ id: string; width: number }>) => {
      panelFor(state, action.payload.id).sidebarWidth = action.payload.width;
    },
  },
  extraReducers: builder => {
    builder.addCase(resetUserScopedState, () => initialState);
  },
});

export const { ensurePanelLayout, setSidebarVisible, toggleSidebar, setSidebarWidth } =
  layoutSlice.actions;

/**
 * Select a panel's geometry, falling back to `DEFAULT_PANEL_LAYOUT` (or the
 * provided overrides) when the id has not been seen yet. Returns a stable
 * shape so callers can destructure without null checks.
 */
export const selectPanelLayout =
  (id: string, defaults?: Partial<PanelLayout>) =>
  (state: { layout?: LayoutState }): PanelLayout =>
    // Optional-chain `layout` so screens render even in minimal test stores
    // that don't wire the (purely cosmetic) layout reducer.
    state.layout?.panels[id] ?? { ...DEFAULT_PANEL_LAYOUT, ...defaults };

export default layoutSlice.reducer;
