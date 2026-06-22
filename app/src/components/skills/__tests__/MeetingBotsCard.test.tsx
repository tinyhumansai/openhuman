import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { MeetCallRecord } from '../../../services/meetCallService';
import {
  setBackendMeetError,
  setBackendMeetJoined,
} from '../../../store/backendMeetSlice';
import { renderWithProviders } from '../../../test/test-utils';
import MeetingBotsCard from '../MeetingBotsCard';

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
    listMock.mockResolvedValue([]);
  });
  afterEach(() => cleanup());

  it('renders the inline form directly (no banner/modal)', () => {
    renderWithProviders(<MeetingBotsCard />);
    expect(screen.getByLabelText(/meeting link/i)).toBeInTheDocument();
  });

  it('submits to joinMeetViaBackendBot and transitions to active view', async () => {
    joinMock.mockResolvedValueOnce({
      meetUrl: 'https://meet.google.com/abc-defg-hij',
      platform: 'gmeet',
    });
    const onToast = vi.fn();
    const { store } = renderWithProviders(<MeetingBotsCard onToast={onToast} />);

    fireEvent.change(screen.getByLabelText(/meeting link/i), {
      target: { value: 'https://meet.google.com/abc-defg-hij' },
    });
    fireEvent.change(screen.getByLabelText(/your name in this meeting/i), {
      target: { value: 'Alice' },
    });
    const form = document.querySelector('form')!;
    fireEvent.submit(form);

    await vi.waitFor(() => {
      expect(joinMock).toHaveBeenCalledWith(
        expect.objectContaining({
          meetUrl: 'https://meet.google.com/abc-defg-hij',
          displayName: 'Tiny',
          platform: 'gmeet',
          agentName: 'Tiny',
          // Participant-name field is wired to the backend authorized-speaker gate.
          respondToParticipant: 'Alice',
          // Active toggle defaults to checked → listen-only false.
          listenOnly: false,
        })
      );
    });
    // Dispatching setBackendMeetJoined transitions the parent MeetingBotsCard from
    // MeetingBotsInline to ActiveMeetingView. The inline component is unmounted at
    // that point, so its useEffect success-toast branch does not fire. Verify the
    // active view is now shown instead.
    store.dispatch(
      setBackendMeetJoined({ meetUrl: 'https://meet.google.com/abc-defg-hij' })
    );
    await vi.waitFor(() => {
      expect(screen.getAllByText(/live/i).length).toBeGreaterThan(0);
    });
  });

  it('uses the saved persona and mascot profile when joining', async () => {
    joinMock.mockResolvedValueOnce({
      meetUrl: 'https://meet.google.com/abc-defg-hij',
      platform: 'gmeet',
    });

    renderWithProviders(<MeetingBotsCard />, {
      preloadedState: {
        persona: { displayName: 'Nova', description: 'Calm and concise.' },
        mascot: {
          color: 'custom',
          voiceId: null,
          voiceGender: 'male',
          voiceUseLocaleDefault: false,
          selectedMascotId: 'yellow',
          customMascotGifUrl: null,
          customPrimaryColor: '#123456',
          customSecondaryColor: '#abcdef',
        },
      },
    });

    fireEvent.change(screen.getByLabelText(/meeting link/i), {
      target: { value: 'https://meet.google.com/abc-defg-hij' },
    });
    fireEvent.submit(document.querySelector('form')!);

    await vi.waitFor(() => {
      expect(joinMock).toHaveBeenCalledWith(
        expect.objectContaining({
          meetUrl: 'https://meet.google.com/abc-defg-hij',
          displayName: 'Nova',
          agentName: 'Nova',
          systemPrompt: 'Calm and concise.',
          mascotId: 'yellow',
          riveColors: { primaryColor: '#123456', secondaryColor: '#abcdef' },
        })
      );
    });
  });

  it('surfaces a join error inline + as an error toast', async () => {
    joinMock.mockRejectedValueOnce(new Error('Bad URL'));
    const onToast = vi.fn();
    renderWithProviders(<MeetingBotsCard onToast={onToast} />);

    fireEvent.change(screen.getByLabelText(/meeting link/i), {
      target: { value: 'https://meet.google.com/x' },
    });
    fireEvent.submit(document.querySelector('form')!);

    await vi.waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'error', title: expect.stringMatching(/not start/i) })
      );
    });
    expect(screen.getByRole('alert')).toHaveTextContent('Bad URL');
  });

  it('surfaces the backend rejection error inline', async () => {
    joinMock.mockResolvedValueOnce({
      meetUrl: 'https://meet.google.com/abc-defg-hij',
      platform: 'gmeet',
    });
    const onToast = vi.fn();
    const { store } = renderWithProviders(<MeetingBotsCard onToast={onToast} />);

    fireEvent.change(screen.getByLabelText(/meeting link/i), {
      target: { value: 'https://meet.google.com/abc-defg-hij' },
    });
    fireEvent.submit(document.querySelector('form')!);

    await vi.waitFor(() => expect(joinMock).toHaveBeenCalled());
    store.dispatch(
      setBackendMeetError({ error: 'Meeting bot is a paid-plan feature.' })
    );

    await vi.waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent(
        'Meeting bot is a paid-plan feature.'
      );
    });
    expect(onToast).toHaveBeenCalledWith(
      expect.objectContaining({
        type: 'error',
        title: expect.stringMatching(/not start/i),
      })
    );
  });

  it('shows the Google Meet CTA button', () => {
    renderWithProviders(<MeetingBotsCard />);
    expect(screen.getByRole('button', { name: /send to google meet/i })).toBeInTheDocument();
  });

  it('asks for the meeting link and the participant the bot answers to', () => {
    renderWithProviders(<MeetingBotsCard />);
    expect(screen.getByLabelText(/meeting link/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/your name in this meeting/i)).toBeInTheDocument();
  });

  it('forwards listen-only when the active toggle is unchecked', async () => {
    joinMock.mockResolvedValueOnce({
      meetUrl: 'https://meet.google.com/abc-defg-hij',
      platform: 'gmeet',
    });
    renderWithProviders(<MeetingBotsCard />);

    fireEvent.change(screen.getByLabelText(/meeting link/i), {
      target: { value: 'https://meet.google.com/abc-defg-hij' },
    });
    fireEvent.change(screen.getByLabelText(/your name in this meeting/i), {
      target: { value: 'Alice' },
    });
    // Active toggle is checked by default; unchecking it selects listen-only.
    fireEvent.click(screen.getByRole('checkbox'));
    fireEvent.submit(document.querySelector('form')!);

    await vi.waitFor(() => {
      expect(joinMock).toHaveBeenCalledWith(expect.objectContaining({ listenOnly: true }));
    });
  });
});

// ── ActiveMeetingView tests ───────────────────────────────────────────────────

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
    expect(screen.getAllByText(/live/i).length).toBeGreaterThan(0);
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

  it('shows the inline form (not ActiveMeetingView) while status is joining', () => {
    renderWithProviders(<MeetingBotsCard />, {
      preloadedState: {
        backendMeet: { ...activeMeetState.backendMeet, status: 'joining' as const },
      },
    });
    expect(screen.getByLabelText(/meeting link/i)).toBeInTheDocument();
    expect(screen.queryByText(/live in meeting/i)).not.toBeInTheDocument();
  });

  it('shows the inline form (not ActiveMeetingView) when status is ended', () => {
    renderWithProviders(<MeetingBotsCard />, {
      preloadedState: { backendMeet: { ...activeMeetState.backendMeet, status: 'ended' as const } },
    });
    expect(screen.getByLabelText(/meeting link/i)).toBeInTheDocument();
    expect(screen.queryByText(/live in meeting/i)).not.toBeInTheDocument();
  });

  it('shows error toast when leave call fails', async () => {
    leaveMock.mockRejectedValueOnce(new Error('Network error'));
    const onToast = vi.fn();
    renderWithProviders(<MeetingBotsCard onToast={onToast} />, { preloadedState: activeMeetState });
    fireEvent.click(screen.getByRole('button', { name: /leave/i }));
    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }))
    );
  });
});

// ── RecentCallsSection / RecentCallRow tests ──────────────────────────────────

function makeCallRecord(overrides: Partial<MeetCallRecord> = {}): MeetCallRecord {
  return {
    request_id: 'req-1',
    meet_url: 'https://meet.google.com/abc-defg-hij',
    bot_display_name: 'OpenHuman',
    owner_display_name: 'Alice',
    started_at_ms: Date.now() - 5 * 60 * 1000,
    ended_at_ms: Date.now() - 4 * 60 * 1000,
    listened_seconds: 30,
    spoken_seconds: 30,
    turn_count: 3,
    ...overrides,
  };
}

describe('MeetingBotsCard — recent calls section', () => {
  afterEach(() => cleanup());

  it('shows a loading hint while listMeetCalls is pending', () => {
    listMock.mockReturnValue(new Promise(() => {}));
    renderWithProviders(<MeetingBotsCard />);
    expect(screen.getByText(/loading…/i)).toBeInTheDocument();
  });

  it('shows an empty-state message when listMeetCalls returns an empty array', async () => {
    listMock.mockResolvedValueOnce([]);
    renderWithProviders(<MeetingBotsCard />);
    await waitFor(() => {
      expect(screen.getByText(/no previous calls yet/i)).toBeInTheDocument();
    });
  });

  it('renders a row for each returned call record', async () => {
    const records = [
      makeCallRecord({
        request_id: 'req-1',
        meet_url: 'https://meet.google.com/aaa-bbbb-ccc',
        turn_count: 2,
      }),
      makeCallRecord({
        request_id: 'req-2',
        meet_url: 'https://meet.google.com/ddd-eeee-fff',
        turn_count: 5,
      }),
    ];
    listMock.mockResolvedValueOnce(records);
    renderWithProviders(<MeetingBotsCard />);
    await waitFor(() => {
      expect(screen.getByText('aaa-bbbb-ccc')).toBeInTheDocument();
      expect(screen.getByText('ddd-eeee-fff')).toBeInTheDocument();
    });
    expect(screen.getByText(/2 turns/i)).toBeInTheDocument();
    expect(screen.getByText(/5 turns/i)).toBeInTheDocument();
  });

  it('renders the owner and participant names on a call row', async () => {
    listMock.mockResolvedValueOnce([
      makeCallRecord({
        owner_display_name: 'Shanu Goyanka',
        participants: ['Shanu Goyanka', 'Alex Rivera'],
      }),
    ]);
    renderWithProviders(<MeetingBotsCard />);
    await waitFor(() => {
      expect(screen.getByText(/added by shanu goyanka/i)).toBeInTheDocument();
    });
    expect(screen.getByText(/with shanu goyanka, alex rivera/i)).toBeInTheDocument();
  });

  it('omits the participants line when the record has none', async () => {
    listMock.mockResolvedValueOnce([
      makeCallRecord({ owner_display_name: '', participants: [] }),
    ]);
    renderWithProviders(<MeetingBotsCard />);
    await waitFor(() => {
      expect(screen.getByText(/3 turns/i)).toBeInTheDocument();
    });
    expect(screen.queryByText(/^with /i)).not.toBeInTheDocument();
    expect(screen.queryByText(/added by/i)).not.toBeInTheDocument();
  });

  it('shows the count badge when there is at least one record', async () => {
    listMock.mockResolvedValueOnce([makeCallRecord()]);
    renderWithProviders(<MeetingBotsCard />);
    await waitFor(() => {
      expect(screen.getByText('(1)')).toBeInTheDocument();
    });
  });

  it('shows an error hint and an empty list when listMeetCalls rejects', async () => {
    listMock.mockRejectedValueOnce(new Error('Network timeout'));
    renderWithProviders(<MeetingBotsCard />);
    await waitFor(() => {
      expect(screen.getByText(/network timeout/i)).toBeInTheDocument();
    });
    expect(screen.queryByText(/loading…/i)).not.toBeInTheDocument();
  });

  it('strips the https://meet.google.com/ prefix and shows only the meeting code', async () => {
    listMock.mockResolvedValueOnce([
      makeCallRecord({ meet_url: 'https://meet.google.com/xyz-1234-abc' }),
    ]);
    renderWithProviders(<MeetingBotsCard />);
    await waitFor(() => {
      expect(screen.getByText('xyz-1234-abc')).toBeInTheDocument();
    });
    expect(screen.queryByText('https://meet.google.com/xyz-1234-abc')).not.toBeInTheDocument();
  });

  it('shows duration as combined spoken + listened seconds', async () => {
    listMock.mockResolvedValueOnce([makeCallRecord({ spoken_seconds: 40, listened_seconds: 20 })]);
    renderWithProviders(<MeetingBotsCard />);
    await waitFor(() => {
      expect(screen.getByText(/60s on call/i)).toBeInTheDocument();
    });
  });

  it('shows a relative timestamp for recent calls', async () => {
    listMock.mockResolvedValueOnce([makeCallRecord({ started_at_ms: Date.now() - 5 * 60 * 1000 })]);
    renderWithProviders(<MeetingBotsCard />);
    await waitFor(() => {
      expect(screen.getByText(/\dm ago/)).toBeInTheDocument();
    });
  });

  it('shows "—" for a zero started_at_ms timestamp', async () => {
    listMock.mockResolvedValueOnce([makeCallRecord({ started_at_ms: 0 })]);
    renderWithProviders(<MeetingBotsCard />);
    await waitFor(() => {
      expect(screen.getByText('—')).toBeInTheDocument();
    });
  });

  it('shows singular "turn" (not "turns") when turn_count is 1', async () => {
    listMock.mockResolvedValueOnce([makeCallRecord({ turn_count: 1 })]);
    renderWithProviders(<MeetingBotsCard />);
    await waitFor(() => {
      expect(screen.getByText(/1 turn$/)).toBeInTheDocument();
    });
    expect(screen.queryByText(/1 turns/)).not.toBeInTheDocument();
  });

  it('falls back to the raw URL when it cannot be parsed', async () => {
    listMock.mockResolvedValueOnce([makeCallRecord({ meet_url: 'not-a-valid-url' })]);
    renderWithProviders(<MeetingBotsCard />);
    await waitFor(() => {
      expect(screen.getByText('not-a-valid-url')).toBeInTheDocument();
    });
  });

  it('shows hours-ago label for a timestamp a few hours old', async () => {
    listMock.mockResolvedValueOnce([
      makeCallRecord({ started_at_ms: Date.now() - 3 * 60 * 60 * 1000 }),
    ]);
    renderWithProviders(<MeetingBotsCard />);
    await waitFor(() => {
      expect(screen.getByText(/3h ago/)).toBeInTheDocument();
    });
  });

  it('shows "yesterday" for a timestamp ~24 hours ago', async () => {
    listMock.mockResolvedValueOnce([
      makeCallRecord({ started_at_ms: Date.now() - 25 * 60 * 60 * 1000 }),
    ]);
    renderWithProviders(<MeetingBotsCard />);
    await waitFor(() => {
      expect(screen.getByText('yesterday')).toBeInTheDocument();
    });
  });

  it('shows Nd-ago label for a timestamp a few days old (< 7)', async () => {
    listMock.mockResolvedValueOnce([
      makeCallRecord({ started_at_ms: Date.now() - 3 * 24 * 60 * 60 * 1000 }),
    ]);
    renderWithProviders(<MeetingBotsCard />);
    await waitFor(() => {
      expect(screen.getByText(/3d ago/)).toBeInTheDocument();
    });
  });

  it('shows a locale date string for a timestamp older than 7 days', async () => {
    listMock.mockResolvedValueOnce([
      makeCallRecord({ started_at_ms: Date.now() - 10 * 24 * 60 * 60 * 1000 }),
    ]);
    renderWithProviders(<MeetingBotsCard />);
    await waitFor(() => {
      const timestamp = screen.queryByText(/ago|yesterday|\dm|\dh/);
      expect(timestamp).not.toBeInTheDocument();
    });
  });
});
