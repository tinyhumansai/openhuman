/**
 * Tests for the recent-calls panel's expandable rows.
 *
 * Confirms a row lazily fetches its transcript + summary on first expand,
 * renders both, and degrades gracefully when the core has no detail or the
 * fetch fails (with a working retry).
 */
import { configureStore } from '@reduxjs/toolkit';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Provider } from 'react-redux';

import { I18nProvider } from '../../lib/i18n/I18nContext';
import type { MeetCallDetail, MeetCallRecord } from '../../services/meetCallService';
import localeReducer from '../../store/localeSlice';
import { RecentCallsSection } from './RecentCallsSection';

const getMeetCallDetail = vi.fn<(requestId: string) => Promise<MeetCallDetail | null>>();

vi.mock('../../services/meetCallService', () => ({
  getMeetCallDetail: (requestId: string) => getMeetCallDetail(requestId),
}));

function call(overrides: Partial<MeetCallRecord> = {}): MeetCallRecord {
  return {
    request_id: 'corr-1',
    meet_url: 'https://meet.google.com/yfj-hcek-zyv',
    bot_display_name: 'Tiny',
    owner_display_name: 'Shanu',
    started_at_ms: Date.now() - 60_000,
    ended_at_ms: Date.now(),
    listened_seconds: 100,
    spoken_seconds: 20,
    turn_count: 3,
    participants: ['Shanu', 'Alan'],
    ...overrides,
  };
}

function renderSection(rows: MeetCallRecord[]) {
  const store = configureStore({
    reducer: { locale: localeReducer },
    preloadedState: { locale: { current: 'en' as const } },
  });
  return render(
    <Provider store={store}>
      <I18nProvider>
        <RecentCallsSection rows={rows} error={null} />
      </I18nProvider>
    </Provider>
  );
}

describe('RecentCallsSection', () => {
  beforeEach(() => {
    getMeetCallDetail.mockReset();
  });
  afterEach(() => {
    vi.clearAllMocks();
  });

  it('lazily loads and renders summary + transcript on expand', async () => {
    getMeetCallDetail.mockResolvedValue({
      request_id: 'corr-1',
      summary: {
        headline: 'Agreed to ship Friday.',
        key_points: ['Ship Friday', 'QA owns sign-off'],
        action_items: [
          { description: 'Send release notes', kind: 'executable', tool_name: 'gmail', assignee: 'Sam' },
        ],
      },
      transcript: [
        { role: 'participant', content: '[00:51] [Shanu] your time' },
        { role: 'assistant', content: '[00:55] [Tiny] On it.' },
      ],
    });

    renderSection([call()]);
    // Not fetched until the row is expanded.
    expect(getMeetCallDetail).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole('button'));

    expect(getMeetCallDetail).toHaveBeenCalledExactlyOnceWith('corr-1');
    await waitFor(() => expect(screen.getByText('Agreed to ship Friday.')).toBeInTheDocument());
    expect(screen.getByText('Summary')).toBeInTheDocument();
    expect(screen.getByText('Ship Friday')).toBeInTheDocument();
    expect(screen.getByText('Transcript')).toBeInTheDocument();
    expect(screen.getByText('[00:55] [Tiny] On it.')).toBeInTheDocument();
    // Action item description + assignee/tool meta.
    expect(screen.getByText('Send release notes')).toBeInTheDocument();
  });

  it('shows an empty state when the call has no recorded detail', async () => {
    getMeetCallDetail.mockResolvedValue(null);
    renderSection([call()]);

    await userEvent.click(screen.getByRole('button'));

    await waitFor(() =>
      expect(
        screen.getByText('No transcript or summary was captured for this call.')
      ).toBeInTheDocument()
    );
  });

  it('surfaces an error with a working retry', async () => {
    getMeetCallDetail.mockRejectedValueOnce(new Error('boom')).mockResolvedValueOnce({
      request_id: 'corr-1',
      summary: null,
      transcript: [{ role: 'participant', content: 'recovered line' }],
    });

    renderSection([call()]);
    await userEvent.click(screen.getByRole('button'));

    const retry = await screen.findByRole('button', { name: 'Retry' });
    await userEvent.click(retry);

    await waitFor(() => expect(screen.getByText('recovered line')).toBeInTheDocument());
    expect(getMeetCallDetail).toHaveBeenCalledTimes(2);
  });

  it('does not refetch when collapsing and re-expanding a fully-loaded call', async () => {
    getMeetCallDetail.mockResolvedValue({
      request_id: 'corr-1',
      summary: { headline: 'All set.', key_points: [], action_items: [] },
      transcript: [{ role: 'participant', content: 'cached line' }],
    });

    renderSection([call()]);
    const toggle = screen.getByRole('button');
    await userEvent.click(toggle); // expand → fetch
    await waitFor(() => expect(screen.getByText('cached line')).toBeInTheDocument());
    await userEvent.click(toggle); // collapse
    await userEvent.click(toggle); // re-expand → reuse cache (summary already present)

    expect(getMeetCallDetail).toHaveBeenCalledTimes(1);
  });

  it('refetches on re-expand to pick up a summary generated after call-end', async () => {
    // First load: transcript persisted, summary still generating (null).
    getMeetCallDetail
      .mockResolvedValueOnce({
        request_id: 'corr-1',
        summary: null,
        transcript: [{ role: 'participant', content: 'early line' }],
      })
      // Second load: summary has since landed.
      .mockResolvedValueOnce({
        request_id: 'corr-1',
        summary: { headline: 'Summary arrived.', key_points: [], action_items: [] },
        transcript: [{ role: 'participant', content: 'early line' }],
      });

    renderSection([call()]);
    const toggle = screen.getByRole('button');
    await userEvent.click(toggle); // expand → first fetch (no summary yet)
    await waitFor(() => expect(screen.getByText('early line')).toBeInTheDocument());
    expect(screen.queryByText('Summary arrived.')).not.toBeInTheDocument();

    await userEvent.click(toggle); // collapse
    await userEvent.click(toggle); // re-expand → refetch (summary was missing)

    await waitFor(() => expect(screen.getByText('Summary arrived.')).toBeInTheDocument());
    expect(getMeetCallDetail).toHaveBeenCalledTimes(2);
  });
});
