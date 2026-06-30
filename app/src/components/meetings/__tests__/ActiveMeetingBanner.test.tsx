import { act, cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { setBackendMeetJoined, setBackendMeetLeft } from '../../../store/backendMeetSlice';
import { renderWithProviders } from '../../../test/test-utils';
import { ActiveMeetingBanner, LEAVE_SAFETY_TIMEOUT_MS } from '../ActiveMeetingBanner';

const leaveMock = vi.fn();

vi.mock('../../../services/meetCallService', async () => {
  const actual = await vi.importActual<typeof import('../../../services/meetCallService')>(
    '../../../services/meetCallService'
  );
  return { ...actual, leaveBackendMeetBot: (...args: unknown[]) => leaveMock(...args) };
});

// RiveMascot is heavy — stub it out
vi.mock('../../../features/human/Mascot', () => ({
  RiveMascot: ({ face }: { face: string }) => <div data-testid="rive-mascot" data-face={face} />,
}));

const joiningState = {
  backendMeet: {
    status: 'joining' as const,
    meetUrl: 'https://meet.google.com/abc-defg-hij',
    meetingId: 'test-id',
    listenOnly: false,
    lastReply: null,
    lastHarness: null,
    transcript: null,
    error: null,
  },
};

const activeState = {
  backendMeet: {
    status: 'active' as const,
    meetUrl: 'https://meet.google.com/abc-defg-hij',
    meetingId: 'test-id',
    listenOnly: false,
    lastReply: null,
    lastHarness: null,
    transcript: null,
    error: null,
  },
};

describe('ActiveMeetingBanner', () => {
  beforeEach(() => {
    leaveMock.mockReset();
  });
  afterEach(() => cleanup());

  it('renders joining state with LIVE badge', () => {
    renderWithProviders(<ActiveMeetingBanner />, { preloadedState: joiningState });

    expect(screen.getByText(/live/i)).toBeInTheDocument();
    expect(screen.getByText(/joining/i)).toBeInTheDocument();
  });

  it('renders active state with Leave button', () => {
    renderWithProviders(<ActiveMeetingBanner />, { preloadedState: activeState });

    expect(screen.getByRole('button', { name: /leave/i })).toBeInTheDocument();
  });

  it('shows meeting code in active state', () => {
    renderWithProviders(<ActiveMeetingBanner />, { preloadedState: activeState });

    expect(screen.getByText(/abc-defg-hij/i)).toBeInTheDocument();
  });

  it('calls leaveBackendMeetBot when Leave is clicked', async () => {
    leaveMock.mockResolvedValueOnce(undefined);
    renderWithProviders(<ActiveMeetingBanner />, { preloadedState: activeState });

    fireEvent.click(screen.getByRole('button', { name: /leave/i }));

    await waitFor(() => {
      expect(leaveMock).toHaveBeenCalledWith('user-requested');
    });
  });

  it('shows a pending "Leaving…" state while the leave request is in flight', async () => {
    // Hold the leave request open so the pending state stays visible.
    let resolveLeave: () => void = () => {};
    leaveMock.mockImplementationOnce(
      () =>
        new Promise<void>(resolve => {
          resolveLeave = resolve;
        })
    );
    renderWithProviders(<ActiveMeetingBanner />, { preloadedState: activeState });

    const leaveBtn = screen.getByRole('button', { name: /leave/i });
    fireEvent.click(leaveBtn);

    // While in flight the label switches to "Leaving…" and the button is disabled.
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /leaving/i })).toBeDisabled();
    });
    expect(screen.queryByRole('button', { name: /^leave$/i })).not.toBeInTheDocument();

    resolveLeave();
  });

  it('keeps the pending state after the request resolves until status changes', async () => {
    leaveMock.mockResolvedValueOnce(undefined);
    const { store } = renderWithProviders(<ActiveMeetingBanner />, { preloadedState: activeState });

    fireEvent.click(screen.getByRole('button', { name: /leave/i }));

    // Even after the request resolves, the pending state holds until the bot leaves.
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /leaving/i })).toBeInTheDocument();
    });

    // Once the meeting ends, the pending Leave button is replaced by Close.
    store.dispatch(setBackendMeetLeft({ reason: 'done' }));
    await waitFor(() => {
      expect(screen.queryByRole('button', { name: /leaving/i })).not.toBeInTheDocument();
      expect(screen.getByRole('button', { name: /close/i })).toBeInTheDocument();
    });
  });

  it('re-enables the Leave button via safety timeout if no left event arrives', async () => {
    vi.useFakeTimers();
    try {
      leaveMock.mockResolvedValueOnce(undefined);
      renderWithProviders(<ActiveMeetingBanner />, { preloadedState: activeState });

      fireEvent.click(screen.getByRole('button', { name: /leave/i }));
      // Flush the resolved leave promise → pending "Leaving…" + safety timer armed.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      expect(screen.getByRole('button', { name: /leaving/i })).toBeDisabled();

      // No status transition arrives; once the safety timeout elapses the button
      // returns to an enabled "Leave" so the user can retry.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(LEAVE_SAFETY_TIMEOUT_MS + 100);
      });
      expect(screen.getByRole('button', { name: /^leave$/i })).toBeEnabled();
    } finally {
      vi.useRealTimers();
    }
  });

  it('clears the pending state and toasts when leave fails', async () => {
    leaveMock.mockRejectedValueOnce(new Error('Network error'));
    const onToast = vi.fn();
    renderWithProviders(<ActiveMeetingBanner onToast={onToast} />, { preloadedState: activeState });

    fireEvent.click(screen.getByRole('button', { name: /leave/i }));

    await waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }));
    });
    // Button returns to the enabled "Leave" state so the user can retry.
    expect(screen.getByRole('button', { name: /^leave$/i })).toBeEnabled();
  });

  it('renders ended state with Close button', () => {
    const { store } = renderWithProviders(<ActiveMeetingBanner />, { preloadedState: activeState });

    store.dispatch(setBackendMeetLeft({ reason: 'done' }));

    // After ended, "Leave" disappears and "Close" appears
    waitFor(() => {
      expect(screen.queryByRole('button', { name: /leave/i })).not.toBeInTheDocument();
      expect(screen.getByRole('button', { name: /close/i })).toBeInTheDocument();
    });
  });

  it('renders error state with Close button', () => {
    const errorState = {
      backendMeet: {
        status: 'error' as const,
        meetUrl: 'https://meet.google.com/abc-defg-hij',
        meetingId: 'test-id',
        listenOnly: false,
        lastReply: null,
        lastHarness: null,
        transcript: null,
        error: 'Failed to connect.',
      },
    };

    renderWithProviders(<ActiveMeetingBanner />, { preloadedState: errorState });

    expect(screen.getByText(/failed to join/i)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /close/i })).toBeInTheDocument();
  });

  it('shows listening status when listenOnly is active', () => {
    const listenState = {
      backendMeet: {
        status: 'active' as const,
        meetUrl: 'https://meet.google.com/abc-defg-hij',
        meetingId: 'test-id',
        listenOnly: true,
        lastReply: null,
        lastHarness: null,
        transcript: null,
        error: null,
      },
    };

    renderWithProviders(<ActiveMeetingBanner />, { preloadedState: listenState });

    expect(screen.getByText(/listening/i)).toBeInTheDocument();
  });

  it('calls onToast with error when leave fails', async () => {
    leaveMock.mockRejectedValueOnce(new Error('Network error'));
    const onToast = vi.fn();

    renderWithProviders(<ActiveMeetingBanner onToast={onToast} />, { preloadedState: activeState });

    fireEvent.click(screen.getByRole('button', { name: /leave/i }));

    await waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }));
    });
  });

  it('transitions mascot face based on state', () => {
    const { store } = renderWithProviders(<ActiveMeetingBanner />, {
      preloadedState: joiningState,
    });

    // Joining → thinking face
    expect(screen.getByTestId('rive-mascot')).toHaveAttribute('data-face', 'thinking');

    // Active → idle (no replies yet)
    store.dispatch(setBackendMeetJoined({ meetUrl: 'https://meet.google.com/abc-defg-hij' }));
    waitFor(() => {
      expect(screen.getByTestId('rive-mascot')).toHaveAttribute('data-face', 'idle');
    });
  });
});
