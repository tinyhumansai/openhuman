import { act, cleanup, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { MeetCallDetail, MeetCallRecord } from '../../../services/meetCallService';
import { renderWithProviders } from '../../../test/test-utils';
import HistoryDetail from '../HistoryDetail';

const getMeetCallDetailMock = vi.fn();

vi.mock('../../../services/meetCallService', async () => {
  const actual = await vi.importActual<typeof import('../../../services/meetCallService')>(
    '../../../services/meetCallService'
  );
  return { ...actual, getMeetCallDetail: (...args: unknown[]) => getMeetCallDetailMock(...args) };
});

// Also mock ActionItemChecklist and TranscriptViewer for isolation
vi.mock('../ActionItemChecklist', () => ({
  default: ({ items }: { items: unknown[] }) => (
    <div data-testid="action-items">action-items:{items.length}</div>
  ),
  ActionItemChecklist: ({ items }: { items: unknown[] }) => (
    <div data-testid="action-items">action-items:{items.length}</div>
  ),
}));

vi.mock('../TranscriptViewer', () => ({
  default: ({ lines }: { lines: unknown[] }) => (
    <div data-testid="transcript">transcript:{lines.length}</div>
  ),
  TranscriptViewer: ({ lines }: { lines: unknown[] }) => (
    <div data-testid="transcript">transcript:{lines.length}</div>
  ),
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const record: MeetCallRecord = {
  request_id: 'req-1',
  meet_url: 'https://meet.google.com/abc-def-ghi',
  bot_display_name: 'OpenHuman',
  owner_display_name: 'Alice',
  started_at_ms: 1700000000000,
  ended_at_ms: 1700000600000,
  listened_seconds: 300,
  spoken_seconds: 300,
  turn_count: 5,
  participants: ['Alice', 'Bob'],
};

const detailWithSummary: MeetCallDetail = {
  request_id: 'req-1',
  summary: {
    headline: 'Great meeting',
    key_points: ['Point 1', 'Point 2'],
    action_items: [
      { description: 'Do something', kind: 'executable', tool_name: 'calendar', assignee: 'Alice' },
    ],
  },
  transcript: [
    { role: 'participant', content: '[0:01] [Alice] Hello' },
    { role: 'assistant', content: 'Hi there' },
  ],
};

const detailNoSummary: MeetCallDetail = {
  request_id: 'req-1',
  summary: null,
  transcript: [{ role: 'participant', content: 'Plain transcript line' }],
};

describe('HistoryDetail', () => {
  it('shows select prompt when record is null', () => {
    renderWithProviders(<HistoryDetail record={null} />);
    expect(
      screen.getByText('Select a call to see its summary and transcript.')
    ).toBeInTheDocument();
  });

  it('shows loading state while detail is being fetched', async () => {
    // Never resolve
    getMeetCallDetailMock.mockReturnValue(new Promise(() => {}));
    renderWithProviders(<HistoryDetail record={record} />);
    expect(await screen.findByText(/loading/i)).toBeInTheDocument();
  });

  it('renders summary and transcript on success', async () => {
    getMeetCallDetailMock.mockResolvedValue(detailWithSummary);
    renderWithProviders(<HistoryDetail record={record} />);
    await waitFor(() => {
      expect(screen.getByTestId('action-items')).toBeInTheDocument();
      expect(screen.getByTestId('transcript')).toBeInTheDocument();
    });
  });

  it('renders key points and headline from summary', async () => {
    getMeetCallDetailMock.mockResolvedValue(detailWithSummary);
    renderWithProviders(<HistoryDetail record={record} />);
    await waitFor(() => {
      expect(screen.getByText('Great meeting')).toBeInTheDocument();
      expect(screen.getByText('Point 1')).toBeInTheDocument();
    });
  });

  it('shows error state when fetch fails', async () => {
    getMeetCallDetailMock.mockRejectedValue(new Error('network error'));
    renderWithProviders(<HistoryDetail record={record} />);
    await waitFor(() => {
      expect(screen.getByText(/retry/i)).toBeInTheDocument();
    });
  });

  it('shows empty state when detail has no summary or transcript', async () => {
    const emptyDetail: MeetCallDetail = { request_id: 'req-1', summary: null, transcript: [] };
    getMeetCallDetailMock.mockResolvedValue(emptyDetail);
    renderWithProviders(<HistoryDetail record={record} />);
    await waitFor(() => {
      expect(screen.getByText(/nothing captured|no transcript|nothing|empty/i)).toBeInTheDocument();
    });
  });

  it('renders meeting code from URL', async () => {
    getMeetCallDetailMock.mockResolvedValue(detailWithSummary);
    renderWithProviders(<HistoryDetail record={record} />);
    // meeting code = pathname stripped of leading slash
    expect(screen.getByText('abc-def-ghi')).toBeInTheDocument();
  });

  it('renders participant count', async () => {
    getMeetCallDetailMock.mockResolvedValue(detailWithSummary);
    renderWithProviders(<HistoryDetail record={record} />);
    await waitFor(() => {
      expect(screen.getByText(/2 participants/)).toBeInTheDocument();
    });
  });

  it('shows transcript without summary when summary is null', async () => {
    getMeetCallDetailMock.mockResolvedValue(detailNoSummary);
    renderWithProviders(<HistoryDetail record={record} />);
    await waitFor(() => {
      expect(screen.getByTestId('transcript')).toBeInTheDocument();
      expect(screen.queryByTestId('action-items')).toBeNull();
    });
  });

  it('reloads when record changes', async () => {
    getMeetCallDetailMock.mockResolvedValue(detailWithSummary);
    const { rerender } = renderWithProviders(<HistoryDetail record={record} />);
    await waitFor(() => expect(getMeetCallDetailMock).toHaveBeenCalledTimes(1));

    const record2: MeetCallRecord = { ...record, request_id: 'req-2' };
    getMeetCallDetailMock.mockResolvedValue({ ...detailWithSummary, request_id: 'req-2' });
    rerender(<HistoryDetail record={record2} />);
    await waitFor(() => expect(getMeetCallDetailMock).toHaveBeenCalledWith('req-2'));
  });

  // ── Stale response guard (#1) ──────────────────────────────────────────────

  it('ignores a stale getMeetCallDetail response when the user switches records quickly', async () => {
    // record1's fetch is intentionally slow so it resolves AFTER record2.
    let resolveRecord1!: (v: MeetCallDetail) => void;
    const record1Detail: MeetCallDetail = {
      request_id: 'req-1',
      summary: { headline: 'Record 1 Stale', key_points: [], action_items: [] },
      transcript: [],
    };
    const record2Detail: MeetCallDetail = {
      request_id: 'req-2',
      summary: { headline: 'Record 2 Fresh', key_points: [], action_items: [] },
      transcript: [],
    };

    // Route mock responses by request_id so call order doesn't matter.
    getMeetCallDetailMock.mockImplementation((reqId: string) => {
      if (reqId === 'req-1') {
        return new Promise<MeetCallDetail>(r => {
          resolveRecord1 = r;
        });
      }
      return Promise.resolve(record2Detail);
    });

    const record1: MeetCallRecord = { ...record, request_id: 'req-1' };
    const record2: MeetCallRecord = { ...record, request_id: 'req-2' };

    const { rerender } = renderWithProviders(<HistoryDetail record={record1} />);

    // Wait for the 0ms timer to fire so the req-1 fetch is actually in-flight.
    await waitFor(() => {
      expect(getMeetCallDetailMock).toHaveBeenCalledWith('req-1');
    });

    // Switch to record2 while record1's fetch is still pending.
    rerender(<HistoryDetail record={record2} />);

    // record2's fetch resolves immediately — wait for its headline to appear.
    await waitFor(() => {
      expect(screen.getByText('Record 2 Fresh')).toBeInTheDocument();
    });

    // Now deliver the stale record1 response.
    await act(async () => {
      resolveRecord1(record1Detail);
      await Promise.resolve(); // flush microtasks
    });

    // Stale data must not overwrite the current record2 display.
    expect(screen.queryByText('Record 1 Stale')).toBeNull();
    expect(screen.getByText('Record 2 Fresh')).toBeInTheDocument();
  });

  // ── Retry-once guard (#2) ──────────────────────────────────────────────────

  it('auto-retry fires at most once per record when detail loads with no summary', async () => {
    vi.useFakeTimers();
    try {
      const noSummaryDetail: MeetCallDetail = {
        request_id: 'req-1',
        summary: null,
        transcript: [],
      };
      getMeetCallDetailMock.mockResolvedValue(noSummaryDetail);

      renderWithProviders(<HistoryDetail record={record} />);

      // Advance past the 0ms initial-fetch timer and flush the async resolution.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(10);
      });
      expect(getMeetCallDetailMock).toHaveBeenCalledTimes(1);

      // Advance 2 s to trigger the one auto-retry.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
      expect(getMeetCallDetailMock).toHaveBeenCalledTimes(2);

      // Advance another 2 s — retry must NOT fire again.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
      expect(getMeetCallDetailMock).toHaveBeenCalledTimes(2);
    } finally {
      vi.useRealTimers();
    }
  });

  // ── Fast switching shows correct transcript (#1 / #3) ─────────────────────

  it('shows the correct transcript and ignores stale content after rapid record switch', async () => {
    let resolveEarly!: (v: MeetCallDetail) => void;

    const earlyDetail: MeetCallDetail = {
      request_id: 'req-early',
      summary: null,
      transcript: [{ role: 'participant', content: 'EARLY STALE CONTENT' }],
    };
    const lateDetail: MeetCallDetail = {
      request_id: 'req-late',
      summary: null,
      transcript: [{ role: 'participant', content: 'LATE FRESH CONTENT' }],
    };

    getMeetCallDetailMock.mockImplementation((reqId: string) => {
      if (reqId === 'req-early') {
        return new Promise(r => {
          resolveEarly = r;
        });
      }
      return Promise.resolve(lateDetail);
    });

    const earlyRecord: MeetCallRecord = {
      ...record,
      request_id: 'req-early',
      meet_url: 'https://meet.google.com/early-abc',
    };
    const lateRecord: MeetCallRecord = {
      ...record,
      request_id: 'req-late',
      meet_url: 'https://meet.google.com/late-xyz',
    };

    const { rerender } = renderWithProviders(<HistoryDetail record={earlyRecord} />);

    // Wait for the early fetch to start.
    await waitFor(() => expect(getMeetCallDetailMock).toHaveBeenCalledWith('req-early'));

    // Switch while early is still in-flight.
    rerender(<HistoryDetail record={lateRecord} />);

    // Late (current) record's transcript should appear.
    await waitFor(() => {
      expect(screen.getByTestId('transcript')).toBeInTheDocument();
    });

    // Deliver the stale early response.
    await act(async () => {
      resolveEarly(earlyDetail);
      await Promise.resolve();
    });

    // Stale early content must not replace the live late content.
    expect(screen.queryByText('EARLY STALE CONTENT')).toBeNull();
  });
});
