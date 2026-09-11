/**
 * Display-only contract for the generic core notification card.
 *
 * The card used to render the meeting auto-join prompt's buttons and dispatch
 * `openhuman.agent_meetings_notification_action`. Both went with the Meet
 * domain, so the card is now a catch-all that must still *show* an
 * action-carrying notification (`NotificationCenter` routes core items only
 * through this branch) while rendering no buttons and calling no RPC.
 */
import { render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import { store } from '../../store';
import { type NotificationItem } from '../../store/notificationSlice';
import CoreNotificationCard from './CoreNotificationCard';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

function makeItem(overrides: Partial<NotificationItem> = {}): NotificationItem {
  return {
    id: 'core:1',
    category: 'system',
    title: 'Something happened',
    body: 'Details about the thing.',
    timestamp: Date.now(),
    read: false,
    actions: [{ actionId: 'do_thing', label: 'Do the thing', payload: {} }],
    ...overrides,
  } as NotificationItem;
}

function renderCard(item: NotificationItem) {
  return render(
    <Provider store={store}>
      <CoreNotificationCard notification={item} />
    </Provider>
  );
}

describe('CoreNotificationCard', () => {
  it('renders the title and body of an action-carrying notification', () => {
    renderCard(makeItem());
    expect(screen.getByTestId('core-notification-card')).toBeInTheDocument();
    expect(screen.getByText('Something happened')).toBeInTheDocument();
    expect(screen.getByTestId('core-notification-body')).toHaveTextContent(
      'Details about the thing.'
    );
  });

  it('renders no action buttons even when the notification carries actions', () => {
    renderCard(makeItem());
    // Regression guard: re-adding buttons here would dispatch an RPC that the
    // core no longer serves, so the click would fail with "unknown method".
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
    expect(screen.queryByText('Do the thing')).not.toBeInTheDocument();
  });
});
