import { screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import { SidebarProvider } from '../../ui';
import AppSidebar from './AppSidebar';

/** `AppSidebar` reads `useSidebar()` — it must render inside a `<SidebarProvider>`. */
function renderAppSidebar(
  options?: Parameters<typeof renderWithProviders>[1],
  providerProps?: { open?: boolean }
) {
  return renderWithProviders(
    <SidebarProvider open={providerProps?.open ?? true}>
      <AppSidebar />
    </SidebarProvider>,
    options
  );
}

// Keep the mount light: the collapsed rail is the unit under test, not the
// header/nav children (SidebarHeader in particular needs the RootShellLayout
// context the harness doesn't provide). SidebarSlot is left real on purpose —
// the harness itself imports SidebarSlotProvider from it.
//
// The sidebar has no footer at all now. It last held a status strip — the
// connectivity dot and the build/version line — and before that Rewards and
// Feedback rows; Rewards became a cloud-gated `NAV_TABS` destination (covered
// by `SidebarNav.test.tsx`) and Feedback a header icon (covered by
// `SidebarHeader.test.tsx`). `ConnectionIndicator` itself is still live — it
// renders on the Home page — so it is only this mount point that went.
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

describe('nav separator visibility', () => {
  it('shows the separator on a route whose region opens with a plain list', () => {
    renderAppSidebar({ initialEntries: ['/settings'] });
    expect(screen.getByTestId('sidebar-nav-separator').className).not.toContain('opacity-0');
  });

  it('hides it on chat, whose region opens with its own outlined button', () => {
    renderAppSidebar({ initialEntries: ['/chat'] });
    expect(screen.getByTestId('sidebar-nav-separator').className).toContain('opacity-0');
  });

  it('hides it on a chat thread route too', () => {
    renderAppSidebar({ initialEntries: ['/chat/abc123'] });
    expect(screen.getByTestId('sidebar-nav-separator').className).toContain('opacity-0');
  });

  it('keeps the separator mounted when hidden', () => {
    // `opacity-0`, never unmounted: its `my-*` is the only gap between the nav
    // group and the projected region, so removing it would collapse the two
    // lists together.
    renderAppSidebar({ initialEntries: ['/chat'] });
    expect(screen.getByTestId('sidebar-nav-separator')).toBeInTheDocument();
  });

  it('does not match a route that merely starts with the same letters', () => {
    renderAppSidebar({ initialEntries: ['/chatter'] });
    expect(screen.getByTestId('sidebar-nav-separator').className).not.toContain('opacity-0');
  });
});
vi.mock('./SidebarHeader', () => ({ default: () => null }));
vi.mock('./SidebarNav', () => ({ default: () => null }));
vi.mock('./SidebarAppRail', () => ({ default: () => null }));

// The `Sidebar` column stays mounted while collapsed (`collapsible="icon"`),
// so `AppSidebar` — not `RootShellLayout` — is what switches to the compact
// rail body. These render inside a collapsed `SidebarProvider` directly,
// bypassing the mocked SidebarHeader/SidebarNav above so the real collapsed
// branch (drag strip, reopen trigger, CollapsedNavRail) is under test.
describe('AppSidebar — collapsed rail', () => {
  beforeEach(() => vi.clearAllMocks());

  it('renders the reopen trigger and collapsed nav rail instead of the header/nav', () => {
    renderAppSidebar({ initialEntries: ['/chat'] }, { open: false });

    expect(screen.getByTestId('root-shell-reopen')).toBeInTheDocument();
    // The primary nav destinations still resolve via CollapsedNavRail.
    expect(screen.getByRole('button', { name: 'nav.chat' })).toBeInTheDocument();
  });

  it('reserves a strip above the reopen trigger for the macOS traffic lights', () => {
    const { container } = renderAppSidebar({ initialEntries: ['/chat'] }, { open: false });
    // A spacer now, not a drag region of its own — the column around it drags.
    expect(container.querySelector('.h-7.w-full.flex-none')).toBeInTheDocument();
  });

  // The drag region is the whole column, not just that strip. `drag.js` drags a
  // bare region only on a direct hit, so the ~12px of column either side of each
  // 32px rail button — the band sitting between the traffic lights and the rail
  // — was dead chrome. Assert the *value* and that the controls are inside it:
  // presence alone passed before the fix and would pass again after a revert.
  it('marks the whole collapsed column as a deep drag region', () => {
    const { container } = renderAppSidebar({ initialEntries: ['/chat'] }, { open: false });
    const region = container.querySelector('[data-tauri-drag-region]') as HTMLElement;
    expect(region.getAttribute('data-tauri-drag-region')).toBe('deep');
    expect(region.className).toContain('h-full');
    // Only `"deep"` reaches descendants, so containment is what makes the
    // margins around these controls draggable. They stay clickable because
    // `isDragRegion` short-circuits on a clickable element first — which is
    // what resolving them by button role here asserts.
    expect(region).toContainElement(screen.getByTestId('root-shell-reopen'));
    expect(region).toContainElement(screen.getByRole('button', { name: 'nav.chat' }));
  });

  it('gives the reopen trigger the expected analytics id and label', () => {
    renderAppSidebar({ initialEntries: ['/chat'] }, { open: false });
    const reopen = screen.getByTestId('root-shell-reopen');
    expect(reopen).toHaveAttribute('data-analytics-id', 'root-shell-reopen-sidebar');
    expect(reopen).toHaveAttribute('aria-label', 'layout.showSidebar');
  });

  it('does not render the reopen trigger while expanded', () => {
    renderAppSidebar({ initialEntries: ['/chat'] }, { open: true });
    expect(screen.queryByTestId('root-shell-reopen')).not.toBeInTheDocument();
  });
});
