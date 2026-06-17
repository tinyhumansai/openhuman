/**
 * Coverage guard for the new `<div role="button">` row wrapper + onKeyDown
 * handler in `Notifications.tsx` (Bug A of #2279). The wrapper replaces a
 * `<button>` because rows now contain `NotificationLinkPill` (also a
 * `<button>`), and nested interactive elements are invalid HTML.
 *
 * Scoped to behavioural assertions: keyboard activation dispatches
 * `markRead` and navigates. The body-rendering path (`<openhuman-link>`
 * parsing, pill, XSS guards) is covered once by `NotificationCard.test.tsx`
 * via the shared `NotificationBody` component.
 */
import { configureStore } from '@reduxjs/toolkit';
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

import notificationsReducer, {
  clearAll,
  type NotificationCategory,
  type NotificationItem,
  notificationReceived,
} from '../../store/notificationSlice';
import Notifications from '../Notifications';

const { navigate } = vi.hoisted(() => ({ navigate: vi.fn() }));

vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return { ...actual, useNavigate: () => navigate };
});

vi.mock('../../lib/notificationRouter', () => ({ resolveSystemRoute: () => '/some-route' }));

vi.mock('../../components/notifications/NotificationCenter', () => ({
  default: () => <div data-testid="notification-center-stub" />,
}));

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

function makeItem(
  id: string,
  body: string,
  overrides: Partial<NotificationItem> = {}
): NotificationItem {
  return {
    id,
    title: 'A title',
    body,
    category: 'system' as NotificationCategory,
    timestamp: Date.now(),
    read: false,
    ...overrides,
  };
}

function renderPage(items: NotificationItem[]) {
  const store = configureStore({
    reducer: { notifications: notificationsReducer },
    preloadedState: {
      notifications: {
        items,
        preferences: {
          messages: true,
          agents: true,
          skills: true,
          system: true,
          meetings: true,
          reminders: true,
          important: true,
        },
        integrationItems: [],
        integrationUnreadCount: 0,
        integrationLoading: false,
        integrationError: null,
      },
    },
  });
  return {
    store,
    ...render(
      <Provider store={store}>
        <MemoryRouter>
          <Notifications />
        </MemoryRouter>
      </Provider>
    ),
  };
}

describe('Notifications page row wrapper', () => {
  it('renders <openhuman-link> body via the shared NotificationBody', () => {
    renderPage([
      makeItem('n-1', '<openhuman-link path="community/discord">Discord</openhuman-link>'),
    ]);

    const bodyEl = screen.getByTestId('notification-item-body');
    // Pill rendered (so the body uses the shared component), raw tag absent.
    expect(within(bodyEl).getByRole('button', { name: /Discord/i })).toBeInTheDocument();
    expect(bodyEl.textContent ?? '').not.toContain('<openhuman-link');
  });

  it('activates a row via Enter and Space keys', () => {
    const { store } = renderPage([makeItem('n-1', 'plain body')]);

    // The row wrapper is the only role=button inside the row (plain body, no pill).
    const row = screen.getByTestId('notification-item');
    const wrapper = within(row).getByRole('button');

    fireEvent.keyDown(wrapper, { key: 'Enter' });
    expect(navigate).toHaveBeenCalledWith('/some-route');
    expect(store.getState().notifications.items[0].read).toBe(true);

    navigate.mockClear();
    fireEvent.keyDown(wrapper, { key: ' ' });
    expect(navigate).toHaveBeenCalledWith('/some-route');
  });

  it('does NOT activate on other keys (Tab, letters)', () => {
    renderPage([makeItem('n-1', 'plain body')]);
    const row = screen.getByTestId('notification-item');
    const wrapper = within(row).getByRole('button');

    fireEvent.keyDown(wrapper, { key: 'Tab' });
    fireEvent.keyDown(wrapper, { key: 'a' });
    expect(navigate).not.toHaveBeenCalled();
  });

  // Bubbling guard: Enter/Space on a focused inner pill must NOT also activate
  // the row. CodeRabbit catch on PR #2339.
  it('does NOT activate a row when keydown bubbles from the inner pill', () => {
    const { store } = renderPage([
      makeItem('n-1', '<openhuman-link path="community/discord">Discord</openhuman-link>'),
    ]);

    const bodyEl = screen.getByTestId('notification-item-body');
    const pill = within(bodyEl).getByRole('button', { name: /Discord/i });
    fireEvent.keyDown(pill, { key: 'Enter' });
    fireEvent.keyDown(pill, { key: ' ' });
    expect(navigate).not.toHaveBeenCalled();
    expect(store.getState().notifications.items[0].read).toBe(false);
  });
});

describe('Notifications page category filter', () => {
  const mixed = () => [
    makeItem('m-1', 'msg one', { category: 'messages' }),
    makeItem('m-2', 'msg two', { category: 'messages' }),
    makeItem('a-1', 'agent one', { category: 'agents' }),
    makeItem('s-1', 'system one', { category: 'system' }),
  ];

  it('renders an All chip plus one chip per category present in the feed', () => {
    renderPage(mixed());

    expect(screen.getByTestId('notif-filter-chip-all')).toBeInTheDocument();
    expect(screen.getByTestId('notif-filter-chip-messages')).toBeInTheDocument();
    expect(screen.getByTestId('notif-filter-chip-agents')).toBeInTheDocument();
    expect(screen.getByTestId('notif-filter-chip-system')).toBeInTheDocument();
    // No dead chips for categories absent from the feed.
    expect(screen.queryByTestId('notif-filter-chip-skills')).not.toBeInTheDocument();
    expect(screen.queryByTestId('notif-filter-chip-meetings')).not.toBeInTheDocument();
  });

  it('shows all items under the default All filter', () => {
    renderPage(mixed());
    expect(screen.getAllByTestId('notification-item')).toHaveLength(4);
    expect(screen.getByTestId('notif-filter-chip-all')).toHaveAttribute('aria-pressed', 'true');
  });

  it('filters the list to the selected category and marks the chip active', () => {
    renderPage(mixed());

    fireEvent.click(screen.getByTestId('notif-filter-chip-messages'));

    expect(screen.getAllByTestId('notification-item')).toHaveLength(2);
    expect(screen.getByTestId('notif-filter-chip-messages')).toHaveAttribute(
      'aria-pressed',
      'true'
    );
    expect(screen.getByTestId('notif-filter-chip-all')).toHaveAttribute('aria-pressed', 'false');
  });

  it('restores the full list when All is reselected', () => {
    renderPage(mixed());

    fireEvent.click(screen.getByTestId('notif-filter-chip-agents'));
    expect(screen.getAllByTestId('notification-item')).toHaveLength(1);

    fireEvent.click(screen.getByTestId('notif-filter-chip-all'));
    expect(screen.getAllByTestId('notification-item')).toHaveLength(4);
  });

  it('does not render the filter row when there are no notifications', () => {
    renderPage([]);
    expect(screen.queryByTestId('notification-category-filter')).not.toBeInTheDocument();
  });

  it('shows alerts.empty (not filterEmpty) when the feed is entirely empty', () => {
    renderPage([]);
    // t() mock returns the key; verify the generic empty key, not the category-filtered one.
    // Two elements carry 'alerts.empty' (header subtext + empty-state body) — both correct.
    expect(screen.getAllByText('alerts.empty').length).toBeGreaterThan(0);
    expect(screen.queryByText('notifications.filterEmpty')).not.toBeInTheDocument();
  });

  it('falls back to All and forgets the selection when the active category drains, even if it reappears', () => {
    const { store } = renderPage([makeItem('m-1', 'msg one', { category: 'messages' })]);

    // Select the only category present.
    fireEvent.click(screen.getByTestId('notif-filter-chip-messages'));
    expect(screen.getByTestId('notif-filter-chip-messages')).toHaveAttribute(
      'aria-pressed',
      'true'
    );

    // Drain the feed — the selected category disappears, so the view falls back
    // to All (the filter row goes away once nothing is present).
    act(() => {
      store.dispatch(clearAll());
    });
    expect(screen.queryByTestId('notification-category-filter')).not.toBeInTheDocument();

    // The same category reappears. The stale 'messages' selection must NOT
    // resurrect — the view stays on All and shows every item.
    act(() => {
      store.dispatch(notificationReceived(makeItem('m-2', 'msg two', { category: 'messages' })));
    });
    expect(screen.getByTestId('notif-filter-chip-all')).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByTestId('notif-filter-chip-messages')).toHaveAttribute(
      'aria-pressed',
      'false'
    );
    expect(screen.getAllByTestId('notification-item')).toHaveLength(1);
  });
});
