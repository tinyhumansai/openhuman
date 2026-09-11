/**
 * Unit tests for RealtimeVoiceControls (#5399) — the flag-gated realtime
 * voice-chat controls on the Human tab. The session hook is mocked so these
 * tests pin the presentational contract only: label per state, the
 * listening/speaking status line, the error alert, and the start/stop wiring.
 */
import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen } from '@testing-library/react';
import type { ReactNode, RefObject } from 'react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import mascotReducer from '../../store/mascotSlice';
import RealtimeVoiceControls from './RealtimeVoiceControls';
import type { RealtimeVoiceAudio } from './voice/amplitudeLipsync';
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
    getOutputVolume: () => 0,
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

function renderControls(
  onSpeakingChange?: (speaking: boolean) => void,
  audioRef?: RefObject<RealtimeVoiceAudio>
) {
  const store = configureStore({ reducer: { mascot: mascotReducer } });
  return render(
    <Provider store={store}>
      <RealtimeVoiceControls onSpeakingChange={onSpeakingChange} audioRef={audioRef} />
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

  // The speaking edge gates the mascot's lip-sync loop on the page
  // (useAmplitudeLipsync's `enabled`), so it must reflect `active && isSpeaking`,
  // not either half alone.
  it('reports not-speaking to onSpeakingChange while idle', () => {
    const onSpeakingChange = vi.fn();
    renderControls(onSpeakingChange);
    expect(onSpeakingChange).toHaveBeenLastCalledWith(false);
  });

  it('reports speaking only when the session is active and the agent speaks', () => {
    const onSpeakingChange = vi.fn();
    session = makeSession({ state: 'active', isSpeaking: true });
    renderControls(onSpeakingChange);
    expect(onSpeakingChange).toHaveBeenLastCalledWith(true);
  });

  it('reports not-speaking when active but the agent is silent', () => {
    const onSpeakingChange = vi.fn();
    session = makeSession({ state: 'active', isSpeaking: false });
    renderControls(onSpeakingChange);
    expect(onSpeakingChange).toHaveBeenLastCalledWith(false);
  });

  // The lip-sync loop reads the SDK accessor and speaking flag straight out of
  // this ref (see useAmplitudeLipsync); a regression that stopped publishing
  // them would freeze the mascot's mouth mid-turn without failing any of the
  // presentational tests above, so pin the wiring directly.
  it('publishes the loudness accessor and speaking flag into audioRef', () => {
    const getOutputVolume = () => 0.5;
    session = makeSession({ state: 'active', isSpeaking: true, getOutputVolume });
    const audioRef: RefObject<RealtimeVoiceAudio> = {
      current: { getOutputVolume: null, speaking: false },
    };
    renderControls(undefined, audioRef);
    expect(audioRef.current?.getOutputVolume).toBe(getOutputVolume);
    expect(audioRef.current?.speaking).toBe(true);
  });

  it('clears the audioRef on unmount cleanup', () => {
    session = makeSession({ state: 'active', isSpeaking: true, getOutputVolume: () => 0.5 });
    const audioRef: RefObject<RealtimeVoiceAudio> = {
      current: { getOutputVolume: null, speaking: false },
    };
    const { unmount } = renderControls(undefined, audioRef);
    expect(audioRef.current?.speaking).toBe(true);

    unmount();
    expect(audioRef.current?.getOutputVolume).toBeNull();
    expect(audioRef.current?.speaking).toBe(false);
  });

  // Ending a session doesn't unmount the control — the card stays on screen and
  // the session hook just returns to 'idle'. The live effect (not the unmount
  // cleanup) has to clear the ref on that transition, or the mascot would keep
  // reading a stale accessor after the agent has gone.
  it('clears the audioRef when the session goes idle while still mounted', () => {
    const store = configureStore({ reducer: { mascot: mascotReducer } });
    const audioRef: RefObject<RealtimeVoiceAudio> = {
      current: { getOutputVolume: null, speaking: false },
    };
    session = makeSession({ state: 'active', isSpeaking: true, getOutputVolume: () => 0.5 });
    const { rerender } = render(
      <Provider store={store}>
        <RealtimeVoiceControls audioRef={audioRef} />
      </Provider>
    );
    expect(audioRef.current?.speaking).toBe(true);

    session = makeSession({ state: 'idle', isSpeaking: false });
    rerender(
      <Provider store={store}>
        <RealtimeVoiceControls audioRef={audioRef} />
      </Provider>
    );
    expect(audioRef.current?.getOutputVolume).toBeNull();
    expect(audioRef.current?.speaking).toBe(false);
  });

  // The agent falling silent mid-session flips only `isSpeaking` — the session
  // stays 'active'. The publication effect must re-run on that edge alone, or a
  // stale `speaking: true` would sit in the ref and freeze the mascot's mouth
  // open. Keeping `active` true here (unlike the goes-idle case above, where the
  // active→idle change would re-run the effect regardless) pins `speaking` in
  // the effect's dependency array specifically.
  it('publishes the speaking edge while the session stays active', () => {
    const store = configureStore({ reducer: { mascot: mascotReducer } });
    const onSpeakingChange = vi.fn();
    const getOutputVolume = () => 0.5;
    const audioRef: RefObject<RealtimeVoiceAudio> = {
      current: { getOutputVolume: null, speaking: false },
    };
    session = makeSession({ state: 'active', isSpeaking: true, getOutputVolume });
    const { rerender } = render(
      <Provider store={store}>
        <RealtimeVoiceControls audioRef={audioRef} onSpeakingChange={onSpeakingChange} />
      </Provider>
    );
    expect(onSpeakingChange).toHaveBeenLastCalledWith(true);
    expect(audioRef.current?.speaking).toBe(true);

    session = makeSession({ state: 'active', isSpeaking: false, getOutputVolume });
    rerender(
      <Provider store={store}>
        <RealtimeVoiceControls audioRef={audioRef} onSpeakingChange={onSpeakingChange} />
      </Provider>
    );
    expect(onSpeakingChange).toHaveBeenLastCalledWith(false);
    expect(audioRef.current?.speaking).toBe(false);
    // The session never closed, so the accessor stays published — only the
    // speaking flag dropped.
    expect(audioRef.current?.getOutputVolume).toBe(getOutputVolume);
  });
});
