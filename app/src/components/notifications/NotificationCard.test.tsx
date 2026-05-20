import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { IntegrationNotification } from '../../types/notifications';
import NotificationCard, { formatNotificationBodyPreview } from './NotificationCard';

function notification(overrides: Partial<IntegrationNotification> = {}): IntegrationNotification {
  return {
    id: 'notif-1',
    provider: 'cron',
    account_id: 'morning_briefing',
    title: 'morning_briefing',
    body: 'Morning briefing ready.',
    raw_payload: {},
    importance_score: 0.65,
    triage_action: 'react',
    triage_reason: 'Scheduled delivery',
    status: 'unread',
    received_at: new Date().toISOString(),
    scored_at: new Date().toISOString(),
    ...overrides,
  };
}

describe('formatNotificationBodyPreview', () => {
  it('strips openhuman-link markup while preserving the label', () => {
    expect(
      formatNotificationBodyPreview(
        'You can <openhuman-link path="community/discord">Report on Discord</openhuman-link>.'
      )
    ).toBe('You can Report on Discord.');
  });
});

describe('NotificationCard', () => {
  it('does not render raw openhuman-link markup in the body preview', () => {
    const rendered = render(
      <NotificationCard
        notification={notification({
          body: 'Something went wrong.\n<openhuman-link path="community/discord">Report on Discord</openhuman-link>',
        })}
        onMarkRead={vi.fn()}
      />
    );

    expect(screen.getByText(/Report on Discord/)).toBeInTheDocument();
    expect(rendered.container.textContent).not.toContain('<openhuman-link');
    expect(rendered.container.textContent).not.toContain('</openhuman-link>');
  });
});
