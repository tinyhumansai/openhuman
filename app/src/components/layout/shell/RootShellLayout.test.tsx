import { act, fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { setSidebarVisible } from '../../../store/layoutSlice';
import { renderWithProviders } from '../../../test/test-utils';
import { SIDEBAR_ICON_WIDTH } from '../../ui';
import RootShellLayout, { APP_SHELL_LAYOUT_ID } from './RootShellLayout';

// Render i18n keys verbatim so assertions don't depend on locale copy.
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));
// macOS/Tauri-gated, and covered by its own spec.
vi.mock('./WindowDragBar', () => ({ default: () => null }));

/** Seeds the persisted `app-shell` geometry the shell reads on mount. */
function withLayout(sidebarVisible: boolean, sidebarWidth: number) {
  return { layout: { panels: { 'app-shell': { sidebarVisible, sidebarWidth } } } };
}

function renderShell(
  props: { unframed?: boolean } = {},
  preloadedState?: ReturnType<typeof withLayout>
) {
  return renderWithProviders(
    <RootShellLayout sidebar={<nav>sidebar body</nav>} {...props}>
      <main>routed page</main>
    </RootShellLayout>,
    preloadedState ? { preloadedState } : undefined
  );
}

function panel(store: { getState: () => { layout: { panels: Record<string, unknown> } } }) {
  return store.getState().layout.panels['app-shell'] as {
    sidebarVisible: boolean;
    sidebarWidth: number;
  };
}

describe('RootShellLayout', () => {
  it('renders the sidebar and the routed content', () => {
    renderShell();
    expect(screen.getByText('sidebar body')).toBeTruthy();
    expect(screen.getByText('routed page')).toBeTruthy();
  });

  it('mounts the routed content inside the content surface', () => {
    renderShell();
    const surface = screen.getByTestId('app-content-surface');
    expect(surface.contains(screen.getByText('routed page'))).toBe(true);
  });

  it('frames the content surface as a card by default', () => {
    renderShell();
    expect(screen.getByTestId('app-content-surface').dataset.unframed).toBeUndefined();
  });

  it('forwards unframed so a live CEF webview gets a square, edge-to-edge pane', () => {
    renderShell({ unframed: true });
    expect(screen.getByTestId('app-content-surface').dataset.unframed).toBe('true');
  });

  it('leaves the resize divider unfilled so the chrome reads as one surface', () => {
    renderShell();
    const divider = screen.getByTestId('root-shell-divider');
    expect(divider.className).toContain('bg-transparent');
    expect(divider.className).not.toContain('bg-surface-strong');
  });

  it('exposes the divider as a keyboard-operable separator', () => {
    renderShell();
    const divider = screen.getByTestId('root-shell-divider');
    expect(divider.getAttribute('role')).toBe('separator');
    expect(divider.getAttribute('aria-orientation')).toBe('vertical');
    expect(divider.tabIndex).toBe(0);
  });

  it('collapses to an icon-width column rather than unmounting the column', () => {
    renderShell();
    expect(screen.getByTestId('root-shell-sidebar')).toHaveAttribute('data-collapsible', 'icon');
  });
});

// The sidebar primitive is driven as a CONTROLLED view over the `layout` slice.
// Uncontrolled state would render identically here, so these assert the round
// trip — store in, store out — rather than what is on screen.
describe('RootShellLayout — redux-controlled geometry', () => {
  it('renders the persisted width rather than the primitive default', () => {
    renderShell({}, withLayout(true, 300));
    expect(screen.getByTestId('root-shell-sidebar').style.width).toBe('300px');
    expect(screen.getByTestId('root-shell-divider').getAttribute('aria-valuenow')).toBe('300');
  });

  it('clamps a persisted width that sits outside the allowed range', () => {
    renderShell({}, withLayout(true, 9000));
    expect(screen.getByTestId('root-shell-sidebar').style.width).toBe('420px');
  });

  it('commits an arrow-key resize step straight to the store', () => {
    const { store } = renderShell({}, withLayout(true, 300));
    const divider = screen.getByTestId('root-shell-divider');

    fireEvent.keyDown(divider, { key: 'ArrowRight' });
    expect(panel(store).sidebarWidth).toBe(316);

    fireEvent.keyDown(divider, { key: 'ArrowLeft' });
    expect(panel(store).sidebarWidth).toBe(300);
  });

  it('clamps arrow-key resize at the minimum width', () => {
    const { store } = renderShell({}, withLayout(true, 188));
    fireEvent.keyDown(screen.getByTestId('root-shell-divider'), { key: 'ArrowLeft' });
    expect(panel(store).sidebarWidth).toBe(188);
  });

  it('holds drag frames locally and persists once the pointer is released', () => {
    const { store } = renderShell({}, withLayout(true, 300));
    const divider = screen.getByTestId('root-shell-divider');

    fireEvent.pointerDown(divider, { clientX: 500 });
    fireEvent.pointerMove(window, { clientX: 540 });

    // Live geometry is on screen…
    expect(screen.getByTestId('root-shell-sidebar').style.width).toBe('340px');
    // …but nothing has been persisted yet: a drag writes one value, not sixty.
    expect(panel(store).sidebarWidth).toBe(300);

    fireEvent.pointerUp(window);
    expect(panel(store).sidebarWidth).toBe(340);
  });

  it('detaches drag listeners when the window loses focus mid-drag', () => {
    renderShell({}, withLayout(true, 300));
    const divider = screen.getByTestId('root-shell-divider');

    fireEvent.pointerDown(divider, { clientX: 500 });
    // Window loses focus before the pointer is released.
    fireEvent.blur(window);
    // Subsequent pointer moves must not affect layout — listeners are detached.
    fireEvent.pointerMove(window, { clientX: 580 });

    expect(screen.getByTestId('root-shell-sidebar').style.width).toBe('300px');
  });

  it('narrows the column to the icon width when collapsed, rather than unmounting it', () => {
    const { store } = renderShell({}, withLayout(false, 300));

    // `collapsible="icon"` — the column stays mounted at a real, non-zero
    // width. The content's own adaptation (e.g. AppSidebar's collapsed rail)
    // is that content's responsibility, not this shell's; the stand-in
    // `sidebar` prop here has none, so it just renders narrowed.
    const sidebar = screen.getByTestId('root-shell-sidebar');
    expect(sidebar).toHaveAttribute('data-state', 'collapsed');
    expect(sidebar.style.width).toBe(`${SIDEBAR_ICON_WIDTH}px`);
    expect(screen.getByText('sidebar body')).toBeTruthy();
    // The resize rail is hidden while collapsed — a fixed icon width isn't
    // draggable.
    expect(screen.queryByTestId('root-shell-divider')).toBeNull();
    // The routed content still renders unaffected.
    expect(screen.getByText('routed page')).toBeTruthy();

    act(() => {
      store.dispatch(setSidebarVisible({ id: APP_SHELL_LAYOUT_ID, visible: true }));
    });

    expect(panel(store).sidebarVisible).toBe(true);
    expect(screen.getByTestId('root-shell-sidebar')).toHaveAttribute('data-state', 'expanded');
    expect(screen.getByTestId('root-shell-sidebar').style.width).toBe('300px');
  });

  it('leaves mod+B to the command registry — the primitive binds no shortcut', () => {
    const { store } = renderShell({}, withLayout(true, 300));
    fireEvent.keyDown(window, { key: 'b', metaKey: true });
    // A second listener here would toggle against the registry's and cancel out.
    expect(panel(store).sidebarVisible).toBe(true);
  });
});
