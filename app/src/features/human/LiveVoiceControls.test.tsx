import { fireEvent, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import LiveVoiceControls from './LiveVoiceControls';
import type { RealtimeVoiceAudio } from './voice/amplitudeLipsync';
import type { LiveVoiceSession } from './voice/live/useLiveVoiceSession';

const session = vi.hoisted(() => ({ current: null as unknown as LiveVoiceSession }));

vi.mock('./voice/live/useLiveVoiceSession', () => ({ useLiveVoiceSession: () => session.current }));

function makeSession(over: Partial<LiveVoiceSession> = {}): LiveVoiceSession {
  return {
    state: 'idle',
    provider: null,
    threadId: null,
    muted: false,
    captions: [],
    partial: { user: null, agent: null },
    toolCalls: [],
    error: null,
    active: false,
    start: vi.fn(async () => undefined),
    stop: vi.fn(),
    sendText: vi.fn(),
    interrupt: vi.fn(),
    toggleMute: vi.fn(),
    setMuted: vi.fn(),
    getOutputVolume: vi.fn(() => 0.4),
    ...over,
  };
}

describe('LiveVoiceControls', () => {
  beforeEach(() => {
    session.current = makeSession();
  });

  it('offers a start button when idle and starts in the given thread', () => {
    renderWithProviders(<LiveVoiceControls threadId="thread-7" />);
    const start = screen.getByTestId('live-voice-start');
    expect(start).toHaveAccessibleName('Start voice chat');
    fireEvent.click(start);
    expect(session.current.start).toHaveBeenCalledWith({ threadId: 'thread-7' });
  });

  it('auto-starts once when the host has a pending request', () => {
    const consume = vi.fn(() => true);
    renderWithProviders(<LiveVoiceControls threadId="t" consumeAutoStart={consume} />);
    expect(consume).toHaveBeenCalled();
    expect(session.current.start).toHaveBeenCalledWith({ threadId: 't' });
  });

  it('does not auto-start without a pending request', () => {
    renderWithProviders(<LiveVoiceControls consumeAutoStart={() => false} />);
    expect(session.current.start).not.toHaveBeenCalled();
  });

  it('shows connecting status', () => {
    session.current = makeSession({ state: 'connecting', active: true });
    renderWithProviders(<LiveVoiceControls />);
    expect(screen.getByTestId('live-voice-status')).toHaveTextContent('Connecting…');
    expect(screen.getByTestId('live-voice-mute')).toBeDisabled();
  });

  it('renders the live call: provider, captions, partials, tool chips, mute and end', () => {
    session.current = makeSession({
      state: 'speaking',
      active: true,
      provider: 'sarvam',
      captions: [
        { role: 'user', text: 'one' },
        { role: 'agent', text: 'two' },
        { role: 'user', text: 'three' },
      ],
      partial: { user: 'typing', agent: 'saying' },
      toolCalls: [
        { callId: 'a', name: 'web_search', status: 'running' },
        { callId: 'b', name: 'gmail', status: 'failed' },
      ],
    });
    renderWithProviders(<LiveVoiceControls />);

    expect(screen.getByTestId('live-voice-provider')).toHaveTextContent('Sarvam AI');
    expect(screen.getByTestId('live-voice-status')).toHaveTextContent('Speaking');
    const captions = screen.getByTestId('live-voice-captions');
    expect(captions).not.toHaveTextContent('one'); // only the last two finals
    expect(captions).toHaveTextContent('two');
    expect(captions).toHaveTextContent('three');
    expect(screen.getByTestId('live-voice-partial-user')).toHaveTextContent('typing');
    expect(screen.getByTestId('live-voice-partial-agent')).toHaveTextContent('saying');

    const chips = screen.getAllByTestId('live-voice-tool-chip');
    expect(chips).toHaveLength(2);
    expect(chips[0]).toHaveAccessibleName('Running web_search');
    expect(chips[1]).toHaveAttribute('data-status', 'failed');

    fireEvent.click(screen.getByTestId('live-voice-mute'));
    expect(session.current.toggleMute).toHaveBeenCalled();
    fireEvent.click(screen.getByTestId('live-voice-end'));
    expect(session.current.stop).toHaveBeenCalled();
  });

  it('shows muted status and an unmute label while muted', () => {
    session.current = makeSession({ state: 'listening', active: true, muted: true, provider: 'x' });
    renderWithProviders(<LiveVoiceControls />);
    expect(screen.getByTestId('live-voice-status')).toHaveTextContent('Muted');
    expect(screen.getByTestId('live-voice-mute')).toHaveAccessibleName('Unmute microphone');
    expect(screen.getByTestId('live-voice-provider')).toHaveTextContent('x');
  });

  it.each([
    ['mic_denied', /Microphone access was denied/],
    ['mic_unavailable', /No microphone is available/],
    ['connection_failed', /Couldn't connect/],
    ['insufficient_credits', /Add credits to continue/],
    ['paid_plan_required', /requires a paid plan/],
    ['provider_down', /provider exploded/],
  ])('shows the %s error with a retry', (code, text) => {
    session.current = makeSession({
      state: 'error',
      error: { code, message: 'provider exploded', fatal: true },
    });
    renderWithProviders(<LiveVoiceControls threadId="t1" />);
    expect(screen.getByRole('alert')).toHaveTextContent(text);
    fireEvent.click(screen.getByTestId('live-voice-retry'));
    expect(session.current.start).toHaveBeenCalledWith({ threadId: 't1' });
  });

  it('publishes lip-sync and phase to the host, and resets them on unmount', () => {
    session.current = makeSession({ state: 'speaking', active: true });
    const audioRef = { current: { getOutputVolume: null, speaking: false } as RealtimeVoiceAudio };
    const onSpeakingChange = vi.fn();
    const onPhaseChange = vi.fn();
    const { unmount } = renderWithProviders(
      <LiveVoiceControls
        audioRef={audioRef}
        onSpeakingChange={onSpeakingChange}
        onPhaseChange={onPhaseChange}
      />
    );
    expect(audioRef.current.speaking).toBe(true);
    expect(audioRef.current.getOutputVolume?.()).toBe(0.4);
    expect(onSpeakingChange).toHaveBeenLastCalledWith(true);
    expect(onPhaseChange).toHaveBeenLastCalledWith('speaking');

    unmount();
    expect(audioRef.current.speaking).toBe(false);
    expect(audioRef.current.getOutputVolume).toBeNull();
    expect(onSpeakingChange).toHaveBeenLastCalledWith(false);
    expect(onPhaseChange).toHaveBeenLastCalledWith('off');
  });

  it('reports phase off for an idle session', () => {
    const onPhaseChange = vi.fn();
    renderWithProviders(<LiveVoiceControls onPhaseChange={onPhaseChange} />);
    expect(onPhaseChange).toHaveBeenCalledWith('off');
  });
});
