import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { setBackendMeetError, setBackendMeetJoined } from '../../../store/backendMeetSlice';
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
          // Active mode must give the backend a wake phrase so it can emit
          // bot:in_call_request when the participant addresses the bot.
          wakePhrase: 'Hey Tiny',
          // Active toggle defaults to checked → listen-only false.
          listenOnly: false,
        })
      );
    });
    // Dispatching setBackendMeetJoined transitions the parent MeetingBotsCard from
    // MeetingBotsInline to ActiveMeetingView. The inline component is unmounted at
    // that point, so its useEffect success-toast branch does not fire. Verify the
    // active view is now shown instead.
    store.dispatch(setBackendMeetJoined({ meetUrl: 'https://meet.google.com/abc-defg-hij' }));
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
          wakePhrase: 'Hey Nova',
          systemPrompt: 'Calm and concise.',
          mascotId: 'yellow',
          riveColors: { primaryColor: '#123456', secondaryColor: '#abcdef' },
        })
      );
    });
  });

  it('falls back to the legacy mascot color for manifest-only mascot ids', async () => {
    joinMock.mockResolvedValueOnce({
      meetUrl: 'https://meet.google.com/abc-defg-hij',
      platform: 'gmeet',
    });

    renderWithProviders(<MeetingBotsCard />, {
      preloadedState: {
        mascot: {
          color: 'yellow',
          voiceId: null,
          voiceGender: 'male',
          voiceUseLocaleDefault: false,
          selectedMascotId: 'river-guide',
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
      expect(joinMock).toHaveBeenCalledWith(expect.objectContaining({ mascotId: 'yellow' }));
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
    store.dispatch(setBackendMeetError({ error: 'Meeting bot is a paid-plan feature.' }));

    await vi.waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent('Meeting bot is a paid-plan feature.');
    });
    expect(onToast).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'error', title: expect.stringMatching(/not start/i) })
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
      expect(joinMock).toHaveBeenCalledWith(
        expect.objectContaining({
          listenOnly: true,
          wakePhrase: undefined,
        })
      );
    });
  });
});

// ── ActiveMeetingView tests ───────────────────────────────────────────────────

const activeMeetState = {
  backendMeet: {
    status: 'active' as const,
    meetUrl: 'https://meet.google.com/abc-defg-hij',
    meetingId: null,
    listenOnly: false,
    lastReply: null,
    lastHarness: null,
    transcript: null,
    liveTranscript: [],
    livePartialIndex: null,
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

  it('shows the active banner (not the inline form) while status is joining', () => {
    // The redesigned composer shows the live banner for 'joining' (not the inline
    // form). The banner shows the LIVE badge and "Joining…" status text. The
    // composer unmounts so there is no meeting-link input while joining.
    renderWithProviders(<MeetingBotsCard />, {
      preloadedState: {
        backendMeet: { ...activeMeetState.backendMeet, status: 'joining' as const },
      },
    });
    expect(screen.queryByLabelText(/meeting link/i)).not.toBeInTheDocument();
    expect(screen.getByText(/joining/i)).toBeInTheDocument();
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

