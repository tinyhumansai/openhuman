import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import { UpcomingTable } from '../UpcomingTable';

// ---------------------------------------------------------------------------
// Mock the service so we control what meetings are returned.
// ---------------------------------------------------------------------------

const listMock = vi.fn();
const joinMock = vi.fn();
const setEventPolicyMock = vi.fn();

vi.mock('../../../services/meetCallService', async () => {
  const actual = await vi.importActual<typeof import('../../../services/meetCallService')>(
    '../../../services/meetCallService'
  );
  return {
    ...actual,
    listUpcomingMeetings: (...args: unknown[]) => listMock(...args),
    joinMeetViaBackendBot: (...args: unknown[]) => joinMock(...args),
    setEventPolicy: (...args: unknown[]) => setEventPolicyMock(...args),
  };
});

// ---------------------------------------------------------------------------
// Fixture data
// ---------------------------------------------------------------------------

const NOW = Date.now();

function makeMeeting(
  overrides: Partial<{
    calendar_event_id: string;
    title: string;
    start_time_ms: number;
    end_time_ms: number;
    meet_url: string | null;
    platform: string | null;
    participant_count: number | null;
    organizer: string | null;
    join_policy: string;
    calendar_source: string;
  }> = {}
) {
  return {
    calendar_event_id: 'evt-1',
    title: 'Weekly Sync',
    start_time_ms: NOW + 60 * 60 * 1000, // 1 hour from now
    end_time_ms: NOW + 90 * 60 * 1000,
    meet_url: 'https://meet.google.com/abc-def-ghi',
    platform: 'gmeet',
    participant_count: 4,
    organizer: 'alice@example.com',
    join_policy: 'ask',
    calendar_source: 'google:alice@example.com',
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('UpcomingTable', () => {
  beforeEach(() => {
    listMock.mockReset();
    joinMock.mockReset();
    setEventPolicyMock.mockReset();
    setEventPolicyMock.mockResolvedValue(undefined);
  });

  afterEach(() => cleanup());

  it('shows loading skeletons while fetching', () => {
    // Let listMock hang indefinitely.
    listMock.mockImplementation(() => new Promise(() => {}));
    renderWithProviders(<UpcomingTable />);
    // Skeletons are animate-pulse rows — table is present.
    expect(screen.getByRole('table')).toBeInTheDocument();
  });

  it('renders the table heading', async () => {
    listMock.mockResolvedValueOnce([]);
    renderWithProviders(<UpcomingTable />);
    // heading key resolves to "Upcoming" in en locale
    await waitFor(() => expect(screen.getByText(/upcoming/i)).toBeInTheDocument());
  });

  it('shows empty state when no meetings are returned', async () => {
    listMock.mockResolvedValueOnce([]);
    renderWithProviders(<UpcomingTable />);
    await waitFor(() => expect(screen.getByText(/no upcoming meetings/i)).toBeInTheDocument());
  });

  it('renders a meeting row with title, platform, and participant count', async () => {
    listMock.mockResolvedValueOnce([makeMeeting({ title: 'Design Review', participant_count: 7 })]);
    renderWithProviders(<UpcomingTable />);
    await waitFor(() => expect(screen.getByText('Design Review')).toBeInTheDocument());
    // Platform label for 'gmeet' → 'Google Meet'
    expect(screen.getByText(/google meet/i)).toBeInTheDocument();
    // participant count
    expect(screen.getByText(/7 participants/i)).toBeInTheDocument();
  });

  it('shows a date-group separator (Today)', async () => {
    listMock.mockResolvedValueOnce([makeMeeting()]);
    renderWithProviders(<UpcomingTable />);
    await waitFor(() => expect(screen.getByText(/today/i)).toBeInTheDocument());
  });

  it('renders the JoinPolicyToggle for each meeting row', async () => {
    listMock.mockResolvedValueOnce([makeMeeting()]);
    renderWithProviders(<UpcomingTable />);
    await waitFor(() => expect(screen.getByRole('radiogroup')).toBeInTheDocument());
    expect(screen.getByRole('radio', { name: /ask/i })).toHaveAttribute('aria-checked', 'true');
  });

  it('shows a "Join" button (not "Join now") for non-imminent meetings', async () => {
    listMock.mockResolvedValueOnce([
      makeMeeting({ start_time_ms: NOW + 60 * 60 * 1000 }), // 1 hour away
    ]);
    renderWithProviders(<UpcomingTable />);
    await waitFor(() => {
      const btn = screen.queryByRole('button', { name: /^join$/i });
      expect(btn).toBeInTheDocument();
    });
    expect(screen.queryByRole('button', { name: /join now/i })).not.toBeInTheDocument();
  });

  it('shows a "Join now" primary button for imminent meetings (< 5 min)', async () => {
    listMock.mockResolvedValueOnce([
      makeMeeting({ start_time_ms: NOW + 2 * 60 * 1000 }), // 2 min away
    ]);
    renderWithProviders(<UpcomingTable />);
    // The button has an aria-label for screen readers ("Join {title}") so
    // we query by visible text content instead of accessible name.
    await waitFor(() => expect(screen.getByText('Join now')).toBeInTheDocument());
  });

  it('shows error state and retry button when fetch fails', async () => {
    listMock.mockRejectedValueOnce(new Error('Network fail'));
    renderWithProviders(<UpcomingTable />);
    // Wait for the error state: the retry button is the reliable indicator
    // (the error text uses a curly apostrophe that a straight-quote regex won't match).
    await waitFor(() => expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument());
    // The error message is also present in the DOM (accept any apostrophe variant).
    expect(screen.getByText(/load upcoming meetings/i)).toBeInTheDocument();
  });

  it('retries on retry button click', async () => {
    listMock
      .mockRejectedValueOnce(new Error('Network fail'))
      .mockResolvedValueOnce([makeMeeting({ title: 'After Retry' })]);

    renderWithProviders(<UpcomingTable />);
    await waitFor(() => screen.getByRole('button', { name: /retry/i }));

    fireEvent.click(screen.getByRole('button', { name: /retry/i }));

    await waitFor(() => expect(screen.getByText('After Retry')).toBeInTheDocument());
  });

  it('renders a refresh button in the header', async () => {
    listMock.mockResolvedValueOnce([]);
    renderWithProviders(<UpcomingTable />);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /refresh/i })).toBeInTheDocument()
    );
  });

  it('calls joinMeetViaBackendBot when Join button is clicked', async () => {
    joinMock.mockResolvedValueOnce({
      meetUrl: 'https://meet.google.com/abc-def-ghi',
      platform: 'gmeet',
    });
    listMock.mockResolvedValueOnce([makeMeeting()]);
    renderWithProviders(<UpcomingTable />);

    const joinBtn = await screen.findByRole('button', { name: /^join$/i });
    fireEvent.click(joinBtn);

    await waitFor(() => expect(joinMock).toHaveBeenCalledOnce());
    expect(joinMock).toHaveBeenCalledWith(
      expect.objectContaining({
        meetUrl: 'https://meet.google.com/abc-def-ghi',
        listenOnly: true,
        correlationId: 'evt-1',
      })
    );
  });

  it('does not show a join button for meetings without a conferencing URL', async () => {
    listMock.mockResolvedValueOnce([makeMeeting({ meet_url: null })]);
    renderWithProviders(<UpcomingTable />);
    await waitFor(() => expect(screen.getByText('Weekly Sync')).toBeInTheDocument());
    expect(screen.queryByRole('button', { name: /^join/i })).not.toBeInTheDocument();
  });

  it('calls setEventPolicy when join policy toggle changes', async () => {
    const meeting = makeMeeting({ join_policy: 'ask' });
    listMock.mockResolvedValueOnce([meeting]);
    renderWithProviders(<UpcomingTable />);
    await waitFor(() => expect(screen.queryByRole('table')).toBeInTheDocument());
    await waitFor(() => expect(listMock).toHaveBeenCalled());

    // Find the Auto segment radio button
    const autoBtn = screen.getByRole('radio', { name: /auto/i });
    fireEvent.click(autoBtn);
    await waitFor(() => expect(setEventPolicyMock).toHaveBeenCalledWith('evt-1', 'auto'));
  });

  it('reverts join policy on setEventPolicy failure', async () => {
    const meeting = makeMeeting({ join_policy: 'ask' });
    listMock.mockResolvedValueOnce([meeting]);
    setEventPolicyMock.mockRejectedValueOnce(new Error('network error'));
    renderWithProviders(<UpcomingTable />);
    await waitFor(() => expect(listMock).toHaveBeenCalled());

    const autoBtn = screen.getByRole('radio', { name: /auto/i });
    fireEvent.click(autoBtn);
    // Wait for rejection and revert
    await waitFor(() => expect(setEventPolicyMock).toHaveBeenCalled());
    // After revert, the "Ask" segment should be active again
    await waitFor(() => {
      const askBtn = screen.getByRole('radio', { name: /ask/i });
      expect(askBtn).toHaveAttribute('aria-checked', 'true');
    });
  });

  it('failed slow request does not clobber a newer successful change (optimistic race)', async () => {
    // The first setEventPolicy call (ask→auto) is slow and will fail.
    // The second call (auto→skip) is fast and succeeds.
    // After the slow failure is finally rejected, the UI must remain on 'skip'
    // — NOT revert back to 'ask'.
    let rejectSlowCall!: (err: Error) => void;
    const slowFailure = new Promise<void>((_, reject) => {
      rejectSlowCall = reject;
    });

    setEventPolicyMock
      .mockImplementationOnce(() => slowFailure) // ask → auto: slow failure
      .mockResolvedValueOnce(undefined); // auto → skip: fast success

    listMock.mockResolvedValueOnce([makeMeeting({ join_policy: 'ask' })]);
    renderWithProviders(<UpcomingTable />);
    await waitFor(() => expect(listMock).toHaveBeenCalled());

    // First change: ask → auto (triggers slow in-flight RPC call)
    const autoBtn = await screen.findByRole('radio', { name: /auto/i });
    fireEvent.click(autoBtn);

    // Second change while first is still in-flight: auto → skip (fast success)
    const skipBtn = screen.getByRole('radio', { name: /skip/i });
    fireEvent.click(skipBtn);

    // Both RPCs were issued
    await waitFor(() => expect(setEventPolicyMock).toHaveBeenCalledTimes(2));

    // Skip should be the active policy now (second change settled)
    expect(screen.getByRole('radio', { name: /skip/i })).toHaveAttribute('aria-checked', 'true');

    // Now the slow first call rejects — with the bug this would revert to 'ask'
    rejectSlowCall(new Error('network timeout'));

    // After rejection settles, skip must STILL be active
    await waitFor(() => {
      expect(screen.getByRole('radio', { name: /skip/i })).toHaveAttribute('aria-checked', 'true');
    });
    expect(screen.getByRole('radio', { name: /ask/i })).toHaveAttribute('aria-checked', 'false');
  });

  // ── watch_calendar hint ────────────────────────────────────────────────────

  it('shows the watch-calendar hint when watchCalendar=false and there are meetings', async () => {
    listMock.mockResolvedValueOnce([makeMeeting()]);
    renderWithProviders(<UpcomingTable watchCalendar={false} />);
    // Wait for meetings to render
    await waitFor(() => expect(screen.getByText('Weekly Sync')).toBeInTheDocument());
    // Hint text from i18n key 'skills.meetingBots.upcoming.watchCalendarHint'
    expect(screen.getByRole('note')).toBeInTheDocument();
  });

  it('does not show the watch-calendar hint when watchCalendar=true', async () => {
    listMock.mockResolvedValueOnce([makeMeeting()]);
    renderWithProviders(<UpcomingTable watchCalendar={true} />);
    await waitFor(() => expect(screen.getByText('Weekly Sync')).toBeInTheDocument());
    expect(screen.queryByRole('note')).not.toBeInTheDocument();
  });

  it('does not show the watch-calendar hint when watchCalendar=null (unknown)', async () => {
    listMock.mockResolvedValueOnce([makeMeeting()]);
    renderWithProviders(<UpcomingTable watchCalendar={null} />);
    await waitFor(() => expect(screen.getByText('Weekly Sync')).toBeInTheDocument());
    expect(screen.queryByRole('note')).not.toBeInTheDocument();
  });

  it('does not show the watch-calendar hint when there are no meetings even if watchCalendar=false', async () => {
    listMock.mockResolvedValueOnce([]);
    renderWithProviders(<UpcomingTable watchCalendar={false} />);
    await waitFor(() => expect(screen.queryByText(/no upcoming meetings/i)).toBeInTheDocument());
    expect(screen.queryByRole('note')).not.toBeInTheDocument();
  });

  // ── Platform filter uses effective (inferred) platform (#8) ───────────────

  it('filters out a meeting whose effective platform (inferred from URL) does not match', async () => {
    // Meeting has no explicit platform but the URL implies gmeet.
    const gmeetInferred = makeMeeting({
      calendar_event_id: 'evt-gmeet',
      title: 'GMeet Inferred',
      platform: null,
      meet_url: 'https://meet.google.com/abc-def-ghi',
    });
    const zoomExplicit = makeMeeting({
      calendar_event_id: 'evt-zoom',
      title: 'Zoom Explicit',
      platform: 'zoom',
      meet_url: 'https://zoom.us/j/456',
    });
    listMock.mockResolvedValueOnce([gmeetInferred, zoomExplicit]);
    renderWithProviders(<UpcomingTable />);

    await waitFor(() => {
      expect(screen.getByText('GMeet Inferred')).toBeInTheDocument();
      expect(screen.getByText('Zoom Explicit')).toBeInTheDocument();
    });

    // The platform filter dropdown should appear (two distinct effective platforms).
    const select = screen.getByRole('combobox');

    // Filter to Google Meet — inferred-gmeet meeting must remain, zoom must go.
    fireEvent.change(select, { target: { value: 'gmeet' } });
    await waitFor(() => {
      expect(screen.getByText('GMeet Inferred')).toBeInTheDocument();
      expect(screen.queryByText('Zoom Explicit')).toBeNull();
    });

    // Filter to Zoom — zoom must appear, inferred-gmeet must go.
    fireEvent.change(select, { target: { value: 'zoom' } });
    await waitFor(() => {
      expect(screen.getByText('Zoom Explicit')).toBeInTheDocument();
      expect(screen.queryByText('GMeet Inferred')).toBeNull();
    });
  });

  it('relative time strings come from i18n (default en locale)', async () => {
    // A meeting ~30 minutes and 30 seconds away → formatWhen should produce
    // "in 30m" via the 'skills.meetingBots.relative.inMinutes' key.
    // The extra 30 s gives headroom so minor test-execution timing drift
    // doesn't push the floor() result down by one.
    listMock.mockResolvedValueOnce([
      makeMeeting({ start_time_ms: Date.now() + 30 * 60 * 1000 + 30 * 1000 }),
    ]);
    renderWithProviders(<UpcomingTable />);
    // Match the en-locale pattern "in Xm" — proves the string came from i18n,
    // not a hardcoded English fallback.
    await waitFor(() => expect(screen.getByText(/^in \d+m$/)).toBeInTheDocument());
  });
});
