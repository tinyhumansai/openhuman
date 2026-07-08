import { act, cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { MeetCallDetail, MeetCallRecord } from '../../../services/meetCallService';
import { setBackendMeetJoined, setBackendMeetLeft } from '../../../store/backendMeetSlice';
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

// Anchor the day-grouped fixtures to noon UTC of their respective days. The
// history rail buckets by UTC calendar day (`utcDayKey`), so a `NOW - 1h`
// timestamp lands in *yesterday's* bucket when the suite runs just after
// 00:00 UTC — CI hit this and the "Today" group vanished. Noon UTC always
// shares its day's key regardless of wall-clock time, making grouping
// deterministic. (No future-filtering in `groupRecords`, so a noon-today
// timestamp that is technically ahead of `now` still groups as Today.)
const noonTodayUtc = (() => {
  const d = new Date();
  d.setUTCHours(12, 0, 0, 0);
  return d.getTime();
})();

const todayCall: MeetCallRecord = {
  request_id: 'req-today',
  meet_url: 'https://meet.google.com/abc-def-ghi',
  bot_display_name: 'OpenHuman',
  owner_display_name: 'Alice',
  started_at_ms: noonTodayUtc,
  ended_at_ms: noonTodayUtc + 600000,
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
  started_at_ms: noonTodayUtc - 86400000,
  ended_at_ms: noonTodayUtc - 86400000 + 600000,
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

  // ── Auto-refresh when a meeting ends (#4341) ──────────────────────────────

  // These two use fake timers so the mount fetch's 1.2s/3s retry batch is fully
  // drained before the mock is changed — otherwise a leftover mount retry could
  // satisfy the assertion (or load the new call early) without the status
  // transition driving it.
  it('re-fetches recent calls when the meeting status transitions to ended', async () => {
    vi.useFakeTimers();
    try {
      listMeetCallsMock.mockResolvedValue([todayCall]);
      const { store } = renderWithProviders(<HistorySection />);

      // Drain the mount fetch + its delayed retries so none linger.
      await act(async () => {
        await vi.runAllTimersAsync();
      });
      expect(screen.getAllByText('abc-def-ghi').length).toBeGreaterThan(0);

      listMeetCallsMock.mockClear();

      // Going active must NOT trigger a refetch — only the end does.
      act(() => {
        store.dispatch(setBackendMeetJoined({ meetUrl: 'https://meet.google.com/abc-def-ghi' }));
      });
      expect(listMeetCallsMock).not.toHaveBeenCalled();

      act(() => {
        store.dispatch(setBackendMeetLeft({ reason: 'left' }));
      });
      await act(async () => {
        await vi.runAllTimersAsync();
      });

      expect(listMeetCallsMock).toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it('auto-selects the just-finished call after a meeting ends', async () => {
    vi.useFakeTimers();
    try {
      const newCall: MeetCallRecord = {
        request_id: 'req-new',
        meet_url: 'https://meet.google.com/new-call-xyz',
        bot_display_name: 'OpenHuman',
        owner_display_name: 'Alice',
        started_at_ms: NOW - 1000,
        ended_at_ms: NOW,
        listened_seconds: 5,
        spoken_seconds: 0,
        turn_count: 2,
        participants: ['Alice'],
      };

      // Initially two calls; the user manually selects the older (yesterday) one.
      listMeetCallsMock.mockResolvedValue([todayCall, yesterdayCall]);
      const { store } = renderWithProviders(<HistorySection />);
      await act(async () => {
        await vi.runAllTimersAsync();
      });

      fireEvent.click(screen.getByText('j/999888'));
      await act(async () => {
        await vi.runAllTimersAsync();
      });
      expect(getMeetCallDetailMock).toHaveBeenCalledWith('req-yesterday');

      getMeetCallDetailMock.mockClear();
      // After the meeting ends the list gains a brand-new call at the top.
      listMeetCallsMock.mockResolvedValue([newCall, todayCall, yesterdayCall]);

      act(() => {
        store.dispatch(setBackendMeetJoined({ meetUrl: newCall.meet_url }));
      });
      act(() => {
        store.dispatch(setBackendMeetLeft({ reason: 'left' }));
      });
      // First drain runs the end-of-meeting refetch, which moves the selection
      // onto the new call. The second drain fires HistoryDetail's selection
      // effect (a setTimeout(0) scheduled at the act boundary) that loads the
      // newly-selected call's detail.
      await act(async () => {
        await vi.runAllTimersAsync();
      });
      await act(async () => {
        await vi.runAllTimersAsync();
      });

      // Selection should jump from the manually-picked older call to the
      // newly-finished one, so its detail is fetched.
      expect(getMeetCallDetailMock).toHaveBeenCalledWith('req-new');
    } finally {
      vi.useRealTimers();
    }
  });
});
