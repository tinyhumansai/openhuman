import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { MeetCallDetail, MeetCallRecord } from '../../../services/meetCallService';
import { renderWithProviders } from '../../../test/test-utils';
import HistorySection from '../HistorySection';

const listMeetCallsMock = vi.fn();
const getMeetCallDetailMock = vi.fn();

vi.mock('../../../services/meetCallService', async () => {
  const actual = await vi.importActual<typeof import('../../../services/meetCallService')>(
    '../../../services/meetCallService'
  );
  return {
    ...actual,
    listMeetCalls: (...args: unknown[]) => listMeetCallsMock(...args),
    getMeetCallDetail: (...args: unknown[]) => getMeetCallDetailMock(...args),
  };
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

// A call is now auto-selected by default, so the detail pane mounts and calls
// getMeetCallDetail in every populated test — give it a benign default.
beforeEach(() => {
  getMeetCallDetailMock.mockResolvedValue(detail);
});

const NOW = Date.now();

const todayCall: MeetCallRecord = {
  request_id: 'req-today',
  meet_url: 'https://meet.google.com/abc-def-ghi',
  bot_display_name: 'OpenHuman',
  owner_display_name: 'Alice',
  started_at_ms: NOW - 3600000,
  ended_at_ms: NOW - 3000000,
  listened_seconds: 300,
  spoken_seconds: 60,
  turn_count: 5,
  participants: ['Alice'],
};

const yesterdayCall: MeetCallRecord = {
  request_id: 'req-yesterday',
  meet_url: 'https://zoom.us/j/999888',
  bot_display_name: 'OpenHuman',
  owner_display_name: 'Bob',
  started_at_ms: NOW - 86400000 - 3600000,
  ended_at_ms: NOW - 86400000 - 3000000,
  listened_seconds: 120,
  spoken_seconds: 30,
  turn_count: 2,
  participants: ['Bob'],
};

const detail: MeetCallDetail = {
  request_id: 'req-today',
  summary: { headline: 'Sync meeting', key_points: [], action_items: [] },
  transcript: [{ role: 'participant', content: 'Hello' }],
};

describe('HistorySection', () => {
  it('shows loading state while fetching', async () => {
    listMeetCallsMock.mockReturnValue(new Promise(() => {}));
    renderWithProviders(<HistorySection />);
    expect(await screen.findByText(/loading/i)).toBeInTheDocument();
  });

  it('shows empty state when no calls returned', async () => {
    listMeetCallsMock.mockResolvedValue([]);
    renderWithProviders(<HistorySection />);
    await waitFor(() => {
      // HistoryRail shows the i18n empty text when all groups have no calls
      expect(
        screen.getByText(/no previous calls yet|your meeting history will appear|no.*call/i)
      ).toBeInTheDocument();
    });
  });

  it('renders grouped calls', async () => {
    listMeetCallsMock.mockResolvedValue([todayCall, yesterdayCall]);
    renderWithProviders(<HistorySection />);
    await waitFor(() => {
      expect(screen.getByText('Today')).toBeInTheDocument();
      expect(screen.getByText('Yesterday')).toBeInTheDocument();
      // abc-def-ghi is auto-selected, so it appears in both the rail and the
      // detail header — assert at least one occurrence.
      expect(screen.getAllByText('abc-def-ghi').length).toBeGreaterThan(0);
      expect(screen.getByText('j/999888')).toBeInTheDocument();
    });
  });

  it('shows detail pane when a call is selected', async () => {
    listMeetCallsMock.mockResolvedValue([todayCall]);
    getMeetCallDetailMock.mockResolvedValue(detail);
    renderWithProviders(<HistorySection />);

    await waitFor(() => {
      // Auto-selected → detail pane fetches it without a manual click.
      expect(getMeetCallDetailMock).toHaveBeenCalledWith('req-today');
    });
  });

  it('filters calls by search query', async () => {
    listMeetCallsMock.mockResolvedValue([todayCall, yesterdayCall]);
    renderWithProviders(<HistorySection />);

    await waitFor(() => {
      expect(screen.getAllByText('abc-def-ghi').length).toBeGreaterThan(0);
    });

    // Search for the Zoom meeting's code (part of the URL path)
    const searchInput = screen.getByRole('searchbox');
    fireEvent.change(searchInput, { target: { value: '999888' } });

    await waitFor(() => {
      expect(screen.queryByText('abc-def-ghi')).toBeNull();
      expect(screen.getAllByText('j/999888').length).toBeGreaterThan(0);
    });
  });

  it('filters calls by platform', async () => {
    listMeetCallsMock.mockResolvedValue([todayCall, yesterdayCall]);
    renderWithProviders(<HistorySection />);

    await waitFor(() => {
      expect(screen.getAllByText('abc-def-ghi').length).toBeGreaterThan(0);
    });

    // Open the compact platform filter menu and pick Google Meet.
    fireEvent.click(screen.getByRole('button', { name: /all platforms/i }));
    fireEvent.click(screen.getByRole('option', { name: /google meet/i }));

    await waitFor(() => {
      expect(screen.getAllByText('abc-def-ghi').length).toBeGreaterThan(0);
      expect(screen.queryByText('j/999888')).toBeNull();
    });
  });

  it('shows error state when listMeetCalls throws', async () => {
    listMeetCallsMock.mockRejectedValue(new Error('network error'));
    renderWithProviders(<HistorySection />);
    await waitFor(() => {
      expect(screen.getByText(/network error/i)).toBeInTheDocument();
    });
  });

  // ── Selection clearing when filter empties list (#4) ──────────────────────

  it('clears the selection when a search query matches no records', async () => {
    listMeetCallsMock.mockResolvedValue([todayCall]);
    renderWithProviders(<HistorySection />);

    // Wait for auto-selection and detail render.
    await waitFor(() => {
      // abc-def-ghi appears in the rail and/or detail pane header.
      expect(screen.getAllByText('abc-def-ghi').length).toBeGreaterThan(0);
    });

    // Search for a term that matches nothing.
    const searchInput = screen.getByRole('searchbox');
    fireEvent.change(searchInput, { target: { value: 'no-match-xyz-999' } });

    // Detail pane should show the "select a call" placeholder when selection clears.
    await waitFor(() => {
      expect(
        screen.getByText('Select a call to see its summary and transcript.')
      ).toBeInTheDocument();
    });
  });
});
