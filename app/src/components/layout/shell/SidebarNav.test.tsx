import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import { AGENT_ACCOUNT_ID } from '../../../utils/accountsFullscreen';
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

  it('clears an active provider selection when clicking the already-active nav item', () => {
    const { store } = renderWithProviders(<SidebarNav />, {
      initialEntries: ['/connections'],
      preloadedState: {
        accounts: {
          accounts: {
            'acct-slack': {
              id: 'acct-slack',
              provider: 'slack',
              label: 'Slack',
              createdAt: '2026-01-01T00:00:00.000Z',
              status: 'open',
            },
          },
          order: ['acct-slack'],
          activeAccountId: 'acct-slack',
          lastActiveAccountId: 'acct-slack',
          messages: {},
          unread: {},
          logs: {},
          overlayOpen: false,
        },
      },
    });

    fireEvent.click(tabButton('Connections'));

    expect(store.getState().accounts.activeAccountId).toBe(AGENT_ACCOUNT_ID);
  });
});
