import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { MeetCallRecord } from '../../../services/meetCallService';
import { renderWithProviders } from '../../../test/test-utils';
import MeetingBotsCard, { MeetingBotsModal } from '../MeetingBotsCard';

const joinMock = vi.fn();
const listMock = vi.fn();
const leaveMock = vi.fn();

vi.mock('../../../services/meetCallService', async () => {
  const actual = await vi.importActual<typeof import('../../../services/meetCallService')>(
    '../../../services/meetCallService'
  );
  return {
    ...actual,
    joinMeetViaBackendBot: (...args: unknown[]) => joinMock(...args),
    listMeetCalls: (...args: unknown[]) => listMock(...args),
    leaveBackendMeetBot: (...args: unknown[]) => leaveMock(...args),
  };
});

describe('MeetingBotsCard', () => {
  beforeEach(() => {
    joinMock.mockReset();
    listMock.mockReset();
    // Default: resolve with empty list so modal renders without flashing errors.
    listMock.mockResolvedValue([]);
  });
  afterEach(() => cleanup());

  it('renders the banner and hides the modal by default', () => {
    renderWithProviders(<MeetingBotsCard />);
    expect(screen.getByTestId('meeting-bots-banner')).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('opens the modal when the banner is clicked', () => {
    renderWithProviders(<MeetingBotsCard />);
    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });

  it('closes the modal on Cancel', () => {
    renderWithProviders(<MeetingBotsCard />);
    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('closes the modal on Escape', () => {
    renderWithProviders(<MeetingBotsCard />);
    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('submits to joinMeetViaBackendBot and fires a success toast', async () => {
    joinMock.mockResolvedValueOnce({
      meetUrl: 'https://meet.google.com/abc-defg-hij',
      platform: 'gmeet',
    });
    const onToast = vi.fn();
    renderWithProviders(<MeetingBotsCard onToast={onToast} />);

    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    fireEvent.change(screen.getByLabelText(/meeting link/i), {
      target: { value: 'https://meet.google.com/abc-defg-hij' },
    });
    const form = screen.getByRole('dialog').querySelector('form')!;
    fireEvent.submit(form);

    await vi.waitFor(() => {
      expect(joinMock).toHaveBeenCalledWith(
        expect.objectContaining({
          meetUrl: 'https://meet.google.com/abc-defg-hij',
          displayName: 'OpenHuman',
          platform: 'gmeet',
          agentName: 'OpenHuman',
        })
      );
    });
    await vi.waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'success', title: expect.stringMatching(/joining/i) })
      );
    });
    // Modal closes on success
    await vi.waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    });
  });

  it('surfaces a join error inline + as an error toast', async () => {
    joinMock.mockRejectedValueOnce(new Error('Bad URL'));
    const onToast = vi.fn();
    renderWithProviders(<MeetingBotsCard onToast={onToast} />);

    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    fireEvent.change(screen.getByLabelText(/meeting link/i), {
      target: { value: 'https://meet.google.com/x' },
    });
    fireEvent.submit(screen.getByRole('dialog').querySelector('form')!);

    await vi.waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'error', title: expect.stringMatching(/not start/i) })
      );
    });
    expect(screen.getByRole('alert')).toHaveTextContent('Bad URL');
  });

  it('Zoom is a live platform — submit is labelled "Send to Zoom", not "coming soon"', () => {
    renderWithProviders(<MeetingBotsCard />);
    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    // Zoom is fully supported via Recall.ai; submit should not say "coming soon".
    fireEvent.click(screen.getByRole('button', { name: /Zoom/ }));
    expect(screen.queryByRole('button', { name: /coming soon/i })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: /send to zoom/i })).toBeInTheDocument();
  });

  it('does not require the old owner-name field for backend Recall joins', () => {
    renderWithProviders(<MeetingBotsCard />);
    fireEvent.click(screen.getByTestId('meeting-bots-banner'));
    expect(screen.queryByLabelText(/your name in the call/i)).not.toBeInTheDocument();
  });
});

// ── ActiveMeetingView tests ───────────────────────────────────────────────────
// Exercises the live-meeting banner rendered when Redux status is active/joining.

const activeMeetState = {
  backendMeet: {
    status: 'active' as const,
    meetUrl: 'https://meet.google.com/abc-defg-hij',
    lastReply: null,
    lastHarness: null,
    transcript: null,
    error: null,
  },
};

describe('MeetingBotsCard — ActiveMeetingView', () => {
  beforeEach(() => {
    leaveMock.mockReset();
    leaveMock.mockResolvedValue(undefined);
  });
  afterEach(() => cleanup());

  it('shows the LIVE badge and meeting code when status is active', () => {
    renderWithProviders(<MeetingBotsCard />, { preloadedState: activeMeetState });
    // Both "Live" (badge) and "Live in meeting" (status text) are present
    expect(screen.getAllByText(/live/i).length).toBeGreaterThan(0);
    // Pathname stripped: shows "abc-defg-hij" not the full URL
    expect(screen.getByText('abc-defg-hij')).toBeInTheDocument();
  });

  it('shows Leave button when status is active', () => {
    renderWithProviders(<MeetingBotsCard />, { preloadedState: activeMeetState });
    expect(screen.getByRole('button', { name: /leave/i })).toBeInTheDocument();
  });

  it('calls leaveBackendMeetBot when Leave is clicked', async () => {
    renderWithProviders(<MeetingBotsCard />, { preloadedState: activeMeetState });
    fireEvent.click(screen.getByRole('button', { name: /leave/i }));
    await waitFor(() => expect(leaveMock).toHaveBeenCalledWith('user-requested'));
  });

  it('Leave button is disabled during in-flight leave call', async () => {
    // Hang the leave call so we can inspect intermediate disabled state
    leaveMock.mockReturnValue(new Promise(() => {}));
    renderWithProviders(<MeetingBotsCard />, { preloadedState: activeMeetState });
    const btn = screen.getByRole('button', { name: /leave/i });
    fireEvent.click(btn);
    await waitFor(() => expect(btn).toBeDisabled());
  });

  it('shows last reply text when lastReply is set', () => {
    renderWithProviders(<MeetingBotsCard />, {
      preloadedState: {
        backendMeet: {
          ...activeMeetState.backendMeet,
          lastReply: { transcript: 'hello', reply: 'Hi there!', emotion: 'happy' },
        },
      },
    });
    expect(screen.getByText(/hi there/i)).toBeInTheDocument();
  });

  it('shows joining status text when status is joining', () => {
    renderWithProviders(<MeetingBotsCard />, {
      preloadedState: {
        backendMeet: { ...activeMeetState.backendMeet, status: 'joining' as const },
      },
    });
    expect(screen.getByText(/joining/i)).toBeInTheDocument();
  });

  it('shows banner (not ActiveMeetingView) when status is ended', () => {
    // MeetingBotsCard only shows ActiveMeetingView for active/joining.
    // When ended the banner is rendered so the user can start a new call.
    renderWithProviders(<MeetingBotsCard />, {
      preloadedState: {
        backendMeet: { ...activeMeetState.backendMeet, status: 'ended' as const },
      },
    });
    expect(screen.getByTestId('meeting-bots-banner')).toBeInTheDocument();
    expect(screen.queryByText(/live in meeting/i)).not.toBeInTheDocument();
  });

  it('shows error toast when leave call fails', async () => {
    leaveMock.mockRejectedValueOnce(new Error('Network error'));
    const onToast = vi.fn();
    renderWithProviders(<MeetingBotsCard onToast={onToast} />, {
      preloadedState: activeMeetState,
    });
    fireEvent.click(screen.getByRole('button', { name: /leave/i }));
    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'error' })
      )
    );
  });
});

// ── RecentCallsSection / RecentCallRow tests ──────────────────────────────────
// These exercise the listMeetCalls integration inside MeetingBotsModal:
// loading state, empty state, error state, and populated list.

function makeCallRecord(overrides: Partial<MeetCallRecord> = {}): MeetCallRecord {
  return {
    request_id: 'req-1',
    meet_url: 'https://meet.google.com/abc-defg-hij',
    bot_display_name: 'OpenHuman',
    owner_display_name: 'Alice',
    started_at_ms: Date.now() - 5 * 60 * 1000, // 5 minutes ago
    ended_at_ms: Date.now() - 4 * 60 * 1000,
    listened_seconds: 30,
    spoken_seconds: 30,
    turn_count: 3,
    ...overrides,
  };
}

describe('MeetingBotsModal — recent calls section', () => {
  afterEach(() => cleanup());

  it('shows a loading hint while listMeetCalls is pending', () => {
    // Never resolves during this test — simulates a slow fetch.
    listMock.mockReturnValue(new Promise(() => {}));

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    expect(screen.getByText(/loading…/i)).toBeInTheDocument();
  });

  it('shows an empty-state message when listMeetCalls returns an empty array', async () => {
    listMock.mockResolvedValueOnce([]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText(/no previous calls yet/i)).toBeInTheDocument();
    });
  });

  it('renders a row for each returned call record', async () => {
    const records = [
      makeCallRecord({ request_id: 'req-1', meet_url: 'https://meet.google.com/aaa-bbbb-ccc', turn_count: 2 }),
      makeCallRecord({ request_id: 'req-2', meet_url: 'https://meet.google.com/ddd-eeee-fff', turn_count: 5 }),
    ];
    listMock.mockResolvedValueOnce(records);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText('aaa-bbbb-ccc')).toBeInTheDocument();
      expect(screen.getByText('ddd-eeee-fff')).toBeInTheDocument();
    });
    // turn counts shown in the row detail line
    expect(screen.getByText(/2 turns/i)).toBeInTheDocument();
    expect(screen.getByText(/5 turns/i)).toBeInTheDocument();
  });

  it('shows the count badge when there is at least one record', async () => {
    listMock.mockResolvedValueOnce([makeCallRecord()]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      // The "(1)" count badge next to the "Recent calls" heading.
      expect(screen.getByText('(1)')).toBeInTheDocument();
    });
  });

  it('shows an error hint and an empty list when listMeetCalls rejects', async () => {
    listMock.mockRejectedValueOnce(new Error('Network timeout'));

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText(/network timeout/i)).toBeInTheDocument();
    });
    // After the error the rows state falls back to [] — no loading hint.
    expect(screen.queryByText(/loading…/i)).not.toBeInTheDocument();
  });

  it('strips the https://meet.google.com/ prefix and shows only the meeting code', async () => {
    listMock.mockResolvedValueOnce([
      makeCallRecord({ meet_url: 'https://meet.google.com/xyz-1234-abc' }),
    ]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText('xyz-1234-abc')).toBeInTheDocument();
    });
    // Full URL should NOT be visible — only the code portion.
    expect(screen.queryByText('https://meet.google.com/xyz-1234-abc')).not.toBeInTheDocument();
  });

  it('shows duration as combined spoken + listened seconds', async () => {
    listMock.mockResolvedValueOnce([
      makeCallRecord({ spoken_seconds: 40, listened_seconds: 20 }),
    ]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText(/60s on call/i)).toBeInTheDocument();
    });
  });

  it('shows a relative timestamp for recent calls', async () => {
    // started 5 minutes ago
    listMock.mockResolvedValueOnce([
      makeCallRecord({ started_at_ms: Date.now() - 5 * 60 * 1000 }),
    ]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText(/\dm ago/)).toBeInTheDocument();
    });
  });

  it('shows "—" for a zero started_at_ms timestamp', async () => {
    listMock.mockResolvedValueOnce([makeCallRecord({ started_at_ms: 0 })]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText('—')).toBeInTheDocument();
    });
  });

  // ── Extra coverage for RecentCallRow / formatRelativeTime branches ──────────

  it('shows singular "turn" (not "turns") when turn_count is 1', async () => {
    listMock.mockResolvedValueOnce([makeCallRecord({ turn_count: 1 })]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText(/1 turn$/)).toBeInTheDocument();
    });
    expect(screen.queryByText(/1 turns/)).not.toBeInTheDocument();
  });

  it('falls back to the raw URL when it cannot be parsed', async () => {
    listMock.mockResolvedValueOnce([makeCallRecord({ meet_url: 'not-a-valid-url' })]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText('not-a-valid-url')).toBeInTheDocument();
    });
  });

  it('shows hours-ago label for a timestamp a few hours old', async () => {
    listMock.mockResolvedValueOnce([
      makeCallRecord({ started_at_ms: Date.now() - 3 * 60 * 60 * 1000 }),
    ]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText(/3h ago/)).toBeInTheDocument();
    });
  });

  it('shows "yesterday" for a timestamp ~24 hours ago', async () => {
    listMock.mockResolvedValueOnce([
      makeCallRecord({ started_at_ms: Date.now() - 25 * 60 * 60 * 1000 }),
    ]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText('yesterday')).toBeInTheDocument();
    });
  });

  it('shows Nd-ago label for a timestamp a few days old (< 7)', async () => {
    listMock.mockResolvedValueOnce([
      makeCallRecord({ started_at_ms: Date.now() - 3 * 24 * 60 * 60 * 1000 }),
    ]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText(/3d ago/)).toBeInTheDocument();
    });
  });

  it('shows a locale date string for a timestamp older than 7 days', async () => {
    listMock.mockResolvedValueOnce([
      makeCallRecord({ started_at_ms: Date.now() - 10 * 24 * 60 * 60 * 1000 }),
    ]);

    renderWithProviders(<MeetingBotsModal onClose={() => {}} />);

    await waitFor(() => {
      // toLocaleDateString returns "Month Day" — just check it's not a relative label.
      const timestamp = screen.queryByText(/ago|yesterday|\dm|\dh/);
      expect(timestamp).not.toBeInTheDocument();
    });
  });
});
