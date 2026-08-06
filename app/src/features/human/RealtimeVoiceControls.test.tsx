/**
 * Unit tests for RealtimeVoiceControls (#5399) — the flag-gated realtime
 * voice-chat controls on the Human tab. The session hook is mocked so these
 * tests pin the presentational contract only: label per state, the
 * listening/speaking status line, the error alert, and the start/stop wiring.
 */
import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import mascotReducer from '../../store/mascotSlice';
import RealtimeVoiceControls from './RealtimeVoiceControls';
import type { RealtimeVoiceSession } from './voice/useRealtimeVoiceSession';

// `@elevenlabs/react`'s ConversationProvider is only a context shell here — pass
// children through so we exercise the real component tree without the SDK.
vi.mock('@elevenlabs/react', () => ({
  ConversationProvider: ({ children }: { children: ReactNode }) => <>{children}</>,
}));

const start = vi.fn();
const stop = vi.fn();
let session: RealtimeVoiceSession;

vi.mock('./voice/useRealtimeVoiceSession', () => ({ useRealtimeVoiceSession: () => session }));

function makeSession(overrides: Partial<RealtimeVoiceSession> = {}): RealtimeVoiceSession {
  return {
    state: 'idle',
    isSpeaking: false,
    mode: 'listening',
    error: null,
    start,
    stop,
    ...overrides,
  };
}

// `useT()` resolves against the bundled `en` map when no provider is mounted,
// so the accessible names below are the real English labels (en.ts).
const LABEL = {
  start: 'Start voice chat',
  stop: 'End voice chat',
  connecting: 'Connecting…',
  listening: 'Listening',
  speaking: 'Speaking',
} as const;

function renderControls() {
  const store = configureStore({ reducer: { mascot: mascotReducer } });
  return render(
    <Provider store={store}>
      <RealtimeVoiceControls />
    </Provider>
  );
}

describe('RealtimeVoiceControls', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    session = makeSession();
  });

  it('shows the start label and no status while idle', () => {
    renderControls();
    const button = screen.getByRole('button', { name: LABEL.start });
    expect(button).toBeEnabled();
    expect(screen.queryByText(LABEL.listening)).not.toBeInTheDocument();
    expect(screen.queryByText(LABEL.speaking)).not.toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('disables the button and shows the connecting label while connecting', () => {
    session = makeSession({ state: 'connecting' });
    renderControls();
    expect(screen.getByRole('button', { name: LABEL.connecting })).toBeDisabled();
  });

  it('shows the stop label and the listening status when active and not speaking', () => {
    session = makeSession({ state: 'active', isSpeaking: false });
    renderControls();
    expect(screen.getByRole('button', { name: LABEL.stop })).toBeEnabled();
    expect(screen.getByText(LABEL.listening)).toBeInTheDocument();
  });

  it('shows the speaking status when the agent is speaking', () => {
    session = makeSession({ state: 'active', isSpeaking: true });
    renderControls();
    expect(screen.getByText(LABEL.speaking)).toBeInTheDocument();
    expect(screen.queryByText(LABEL.listening)).not.toBeInTheDocument();
  });

  it('surfaces the session error in an alert', () => {
    session = makeSession({ state: 'error', error: 'microphone blocked' });
    renderControls();
    expect(screen.getByRole('alert')).toHaveTextContent('microphone blocked');
  });

  it('starts a session when clicked while idle', () => {
    renderControls();
    fireEvent.click(screen.getByRole('button', { name: LABEL.start }));
    expect(start).toHaveBeenCalledTimes(1);
    expect(stop).not.toHaveBeenCalled();
  });

  it('stops the session when clicked while active', () => {
    session = makeSession({ state: 'active' });
    renderControls();
    fireEvent.click(screen.getByRole('button', { name: LABEL.stop }));
    expect(stop).toHaveBeenCalledTimes(1);
    expect(start).not.toHaveBeenCalled();
  });
});
