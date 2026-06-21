import { screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import SidebarNav from './SidebarNav';

// Analytics is fire-and-forget; stub it so the nav renders without a transport.
vi.mock('../../../services/analytics', () => ({ trackEvent: vi.fn() }));

/** The rendered button for a nav label (label text lives in a child span). */
function tabButton(label: string): HTMLButtonElement {
  return screen.getByRole('button', { name: new RegExp(label) }) as HTMLButtonElement;
}

describe('SidebarNav active matching', () => {
  it('keeps Tiny.Place active on its redirected /agent-world/explore route', () => {
    // The tab links to /agent-world but the index immediately redirects to
    // /agent-world/explore — an exact match would never light up.
    renderWithProviders(<SidebarNav />, { initialEntries: ['/agent-world/explore'] });

    expect(tabButton('Tiny.Place')).toHaveAttribute('aria-current', 'page');
  });

  it('keeps Tiny.Place active on a nested section route', () => {
    renderWithProviders(<SidebarNav />, { initialEntries: ['/agent-world/messaging'] });

    expect(tabButton('Tiny.Place')).toHaveAttribute('aria-current', 'page');
  });

  it('does not mark Tiny.Place active on an unrelated route', () => {
    renderWithProviders(<SidebarNav />, { initialEntries: ['/chat'] });

    expect(tabButton('Tiny.Place')).not.toHaveAttribute('aria-current');
    expect(tabButton('Chat')).toHaveAttribute('aria-current', 'page');
  });
});
