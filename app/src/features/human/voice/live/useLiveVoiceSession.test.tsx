import { act, renderHook, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { Provider } from 'react-redux';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { FakeAudioContext, FakeWebSocket, pcmBytes } from '../../../../test/liveVoiceFakes';
import { createTestStore } from '../../../../test/test-utils';
import { READBACK_PREFIX } from './readbackPrefix';
import { useLiveVoiceSession } from './useLiveVoiceSession';

const mocks = vi.hoisted(() => ({
  socketHandlers: new Map<string, (payload: unknown) => void>(),
  socketId: 'sock-1' as string | undefined,
  capture: {
    stop: vi.fn(),
    setMuted: vi.fn(),
    onFrame: null as null | ((frame: ArrayBuffer) => void),
  },
  startCapture: vi.fn(),
  track: vi.fn(),
  loadThreadMessages: vi.fn((id: string) => ({ type: 'test/loadThreadMessages', payload: id })),
  resolveUrl: vi.fn(async () => 'ws://127.0.0.1:7788/ws/live-voice?token=t'),
}));

vi.mock('../../../../services/socketService', () => ({
  socketService: {
    on: (event: string, cb: (payload: unknown) => void) => mocks.socketHandlers.set(event, cb),
    off: (event: string) => mocks.socketHandlers.delete(event),
    getSocket: () => (mocks.socketId ? { id: mocks.socketId } : null),
  },
}));

vi.mock('../../../../components/analytics', () => ({ trackAnalyticsEvent: mocks.track }));

vi.mock('../../../../store/threadSlice', async importOriginal => {
  const actual = await importOriginal<typeof import('../../../../store/threadSlice')>();
  return { ...actual, loadThreadMessages: mocks.loadThreadMessages };
});

vi.mock('./pcmCaptureWorklet', async importOriginal => {
  const actual = await importOriginal<typeof import('./pcmCaptureWorklet')>();
  return { ...actual, startPcmCapture: mocks.startCapture };
});

vi.mock('./liveVoiceSocket', async importOriginal => {
  const actual = await importOriginal<typeof import('./liveVoiceSocket')>();
  return { ...actual, resolveLiveVoiceUrl: mocks.resolveUrl };
});

let audioContexts: FakeAudioContext[] = [];

function renderSession() {
  const store = createTestStore();
  const wrapper = ({ children }: { children: ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  return renderHook(() => useLiveVoiceSession(), { wrapper });
}

async function startAndReady(
  result: { current: ReturnType<typeof useLiveVoiceSession> },
  opts: { provider?: string | null; threadId?: string | null } = { threadId: 'thread-1' }
) {
  await act(async () => {
    await result.current.start(opts);
  });
  const ws = FakeWebSocket.latest();
  act(() => ws.open());
  await act(async () => {
    ws.emit({
      type: 'ready',
      session_id: 's-1',
      provider: 'gemini-hosted',
      output_sample_rate: 24_000,
      thread_id: 'thread-1',
    });
  });
  await waitFor(() => expect(result.current.state).toBe('listening'));
  return ws;
}

describe('useLiveVoiceSession', () => {
  beforeEach(() => {
    FakeWebSocket.instances = [];
    audioContexts = [];
    vi.stubGlobal('WebSocket', FakeWebSocket);
    vi.stubGlobal(
      'AudioContext',
      class {
        constructor() {
          const ctx = new FakeAudioContext();
          audioContexts.push(ctx);
          return ctx;
        }
      }
    );
    mocks.socketHandlers.clear();
    mocks.socketId = 'sock-1';
    mocks.capture.stop.mockReset();
    mocks.capture.setMuted.mockReset();
    mocks.track.mockReset();
    mocks.loadThreadMessages.mockClear();
    mocks.resolveUrl.mockClear();
    mocks.startCapture.mockReset();
    mocks.startCapture.mockImplementation(async (opts: { onFrame: (f: ArrayBuffer) => void }) => {
      mocks.capture.onFrame = opts.onFrame;
      return { stop: mocks.capture.stop, setMuted: mocks.capture.setMuted };
    });
  });

  afterEach(() => vi.unstubAllGlobals());

  it('goes idle → connecting → listening and sends the start frame', async () => {
    const { result } = renderSession();
    expect(result.current.state).toBe('idle');

    let pending: Promise<void>;
    act(() => {
      pending = result.current.start({ provider: 'sarvam', threadId: 'thread-1' });
    });
    expect(result.current.state).toBe('connecting');
    expect(result.current.active).toBe(true);
    await act(async () => {
      await pending;
    });

    const ws = FakeWebSocket.latest();
    act(() => ws.open());
    expect(ws.jsonFrames()[0]).toEqual({
      type: 'start',
      provider: 'sarvam',
      thread_id: 'thread-1',
      input_sample_rate: 16_000,
      client_id: 'sock-1',
    });
    // The mic is not requested before the core says ready.
    expect(mocks.startCapture).not.toHaveBeenCalled();

    await act(async () => {
      ws.emit({
        type: 'ready',
        session_id: 's',
        provider: 'sarvam',
        output_sample_rate: 22_050,
        thread_id: 'thread-9',
      });
    });
    await waitFor(() => expect(result.current.state).toBe('listening'));
    expect(result.current.provider).toBe('sarvam');
    expect(result.current.threadId).toBe('thread-9');
    expect(mocks.track).toHaveBeenCalledWith('live_voice_session_started', { provider: 'sarvam' });
  });

  it('omits client_id when the chat socket is not connected and sends null defaults', async () => {
    mocks.socketId = undefined;
    const { result } = renderSession();
    await act(async () => {
      await result.current.start();
    });
    const ws = FakeWebSocket.latest();
    act(() => ws.open());
    expect(ws.jsonFrames()[0]).toEqual({
      type: 'start',
      provider: null,
      thread_id: null,
      input_sample_rate: 16_000,
    });
  });

  it('uplinks mic frames unless muted', async () => {
    const { result } = renderSession();
    const ws = await startAndReady(result);
    const frame = new ArrayBuffer(3_200);
    act(() => mocks.capture.onFrame?.(frame));
    expect(ws.sent).toContain(frame);

    act(() => result.current.toggleMute());
    expect(result.current.muted).toBe(true);
    expect(mocks.capture.setMuted).toHaveBeenLastCalledWith(true);
    const muted = new ArrayBuffer(3_200);
    act(() => mocks.capture.onFrame?.(muted));
    expect(ws.sent).not.toContain(muted);

    act(() => result.current.setMuted(false));
    expect(result.current.muted).toBe(false);
  });

  it('replaces partial captions until final, then refreshes the thread', async () => {
    const { result } = renderSession();
    const ws = await startAndReady(result);

    act(() => ws.emit({ type: 'transcript', role: 'user', text: 'Hel', final: false }));
    act(() => ws.emit({ type: 'transcript', role: 'user', text: 'Hello the', final: false }));
    expect(result.current.partial.user).toBe('Hello the');
    expect(result.current.captions).toEqual([]);
    expect(mocks.loadThreadMessages).not.toHaveBeenCalled();

    act(() => ws.emit({ type: 'transcript', role: 'user', text: 'Hello there', final: true }));
    expect(result.current.partial.user).toBeNull();
    expect(result.current.captions).toEqual([{ role: 'user', text: 'Hello there' }]);
    expect(mocks.loadThreadMessages).toHaveBeenCalledWith('thread-1');

    act(() => ws.emit({ type: 'transcript', role: 'agent', text: 'Hi!', final: false }));
    expect(result.current.partial.agent).toBe('Hi!');
    act(() => ws.emit({ type: 'transcript', role: 'agent', text: '  ', final: true }));
    expect(result.current.captions).toHaveLength(1);
  });

  it('plays agent audio, flips to speaking, and flushes on interrupted', async () => {
    const { result } = renderSession();
    const ws = await startAndReady(result);
    act(() => ws.emitRaw(pcmBytes(2_400, 0.3)));
    expect(result.current.state).toBe('speaking');

    const ctx = audioContexts[0];
    ctx.advanceTo(0.05);
    expect(result.current.getOutputVolume()).toBeGreaterThan(0);

    act(() => ws.emit({ type: 'transcript', role: 'agent', text: 'so the', final: false }));
    act(() => ws.emit({ type: 'interrupted' }));
    expect(ctx.sources[0].stopped).toBe(true);
    expect(result.current.state).toBe('listening');
    expect(result.current.partial.agent).toBeNull();
    expect(result.current.getOutputVolume()).toBe(0);
  });

  it('interrupt() flushes locally and tells the core', async () => {
    const { result } = renderSession();
    const ws = await startAndReady(result);
    act(() => ws.emitRaw(pcmBytes(240)));
    act(() => result.current.interrupt());
    expect(audioContexts[0].sources[0].stopped).toBe(true);
    expect(ws.jsonFrames().at(-1)).toEqual({ type: 'interrupt' });
  });

  it('tracks tool chips and prunes finished ones on turn_complete', async () => {
    const { result } = renderSession();
    const ws = await startAndReady(result);
    act(() => {
      ws.emit({ type: 'tool_started', call_id: 'c1', name: 'web_search' });
      ws.emit({ type: 'tool_started', call_id: 'c2', name: 'gmail' });
      ws.emit({ type: 'tool_started', call_id: 'c3', name: 'calendar' });
    });
    expect(result.current.toolCalls.map(c => c.status)).toEqual(['running', 'running', 'running']);

    act(() => {
      ws.emit({
        type: 'tool_finished',
        call_id: 'c1',
        name: 'web_search',
        ok: true,
        cancelled: false,
      });
      ws.emit({ type: 'tool_finished', call_id: 'c2', name: 'gmail', ok: false, cancelled: false });
      ws.emit({ type: 'tool_finished', call_id: 'zz', name: 'late', ok: false, cancelled: true });
    });
    expect(result.current.toolCalls).toEqual([
      { callId: 'c1', name: 'web_search', status: 'ok' },
      { callId: 'c2', name: 'gmail', status: 'failed' },
      { callId: 'c3', name: 'calendar', status: 'running' },
      { callId: 'zz', name: 'late', status: 'cancelled' },
    ]);

    act(() => ws.emit({ type: 'turn_complete' }));
    expect(result.current.toolCalls).toEqual([
      { callId: 'c3', name: 'calendar', status: 'running' },
    ]);
  });

  it('sendText sends a text frame only once ready, trimmed', async () => {
    const { result } = renderSession();
    const ws = await startAndReady(result);
    act(() => result.current.sendText('  what time is it  '));
    act(() => result.current.sendText('   '));
    expect(ws.jsonFrames().filter(f => f.type === 'text')).toEqual([
      { type: 'text', text: 'what time is it' },
    ]);
  });

  it('reads a voice_speak result back once per session with the read-back prefix', async () => {
    const { result } = renderSession();
    const handlerBeforeReady = mocks.socketHandlers.get('voice_speak');
    act(() => handlerBeforeReady?.({ full_response: 'ignored' }));

    const ws = await startAndReady(result);
    const speak = mocks.socketHandlers.get('voice_speak')!;
    act(() => speak({ full_response: '  Your summary.  ' }));
    act(() => speak({ full_response: 'Your summary.' }));
    act(() => speak({ full_response: '' }));
    act(() => speak(undefined));
    const texts = ws.jsonFrames().filter(f => f.type === 'text');
    expect(texts).toEqual([{ type: 'text', text: `${READBACK_PREFIX}\n\nYour summary.` }]);
  });

  it('stop() sends stop, releases mic and audio, and tracks the end', async () => {
    const { result } = renderSession();
    const ws = await startAndReady(result);
    mocks.loadThreadMessages.mockClear();
    act(() => result.current.stop());
    expect(ws.jsonFrames().at(-1)).toEqual({ type: 'stop' });
    expect(ws.readyState).toBe(FakeWebSocket.CLOSED);
    expect(mocks.capture.stop).toHaveBeenCalled();
    expect(audioContexts[0].close).toHaveBeenCalled();
    expect(result.current.state).toBe('idle');
    expect(result.current.active).toBe(false);
    expect(mocks.track).toHaveBeenCalledWith('live_voice_session_ended', {
      provider: 'gemini-hosted',
    });
    expect(mocks.loadThreadMessages).toHaveBeenCalledWith('thread-1');
  });

  it('tears everything down on unmount', async () => {
    const { result, unmount } = renderSession();
    const ws = await startAndReady(result);
    unmount();
    expect(ws.readyState).toBe(FakeWebSocket.CLOSED);
    expect(mocks.capture.stop).toHaveBeenCalled();
    expect(audioContexts[0].close).toHaveBeenCalled();
    expect(mocks.socketHandlers.has('voice_speak')).toBe(false);
  });

  it('a core `closed` event ends the session quietly and refreshes the thread', async () => {
    const { result } = renderSession();
    const ws = await startAndReady(result);
    mocks.loadThreadMessages.mockClear();
    act(() => ws.emit({ type: 'closed', reason: 'idle_timeout' }));
    expect(result.current.state).toBe('idle');
    expect(result.current.error).toBeNull();
    expect(mocks.capture.stop).toHaveBeenCalled();
    expect(mocks.loadThreadMessages).toHaveBeenCalledWith('thread-1');
  });

  it('a socket drop after ready returns to idle', async () => {
    const { result } = renderSession();
    const ws = await startAndReady(result);
    act(() => ws.drop());
    expect(result.current.state).toBe('idle');
  });

  it('a socket that closes before ready is a connection error', async () => {
    const { result } = renderSession();
    await act(async () => {
      await result.current.start({ threadId: 't' });
    });
    act(() => FakeWebSocket.latest().drop(1008));
    expect(result.current.state).toBe('error');
    expect(result.current.error).toMatchObject({ code: 'connection_failed', fatal: true });
  });

  it('non-fatal errors are surfaced without ending the call; fatal ones end it', async () => {
    const { result } = renderSession();
    const ws = await startAndReady(result);
    act(() => ws.emit({ type: 'error', code: 'tool_slow', message: 'slow', fatal: false }));
    expect(result.current.state).toBe('listening');
    expect(result.current.error).toEqual({ code: 'tool_slow', message: 'slow', fatal: false });

    act(() => ws.emit({ type: 'error', code: 'provider_down', message: 'down', fatal: true }));
    expect(result.current.state).toBe('error');
    expect(result.current.error?.code).toBe('provider_down');
    expect(mocks.capture.stop).toHaveBeenCalled();
    expect(result.current.active).toBe(false);
  });

  it('maps a denied microphone to mic_denied', async () => {
    mocks.startCapture.mockRejectedValueOnce(new DOMException('nope', 'NotAllowedError'));
    const { result } = renderSession();
    await act(async () => {
      await result.current.start();
    });
    const ws = FakeWebSocket.latest();
    act(() => ws.open());
    await act(async () => {
      ws.emit({
        type: 'ready',
        session_id: 's',
        provider: 'gemini',
        output_sample_rate: 24_000,
        thread_id: null,
      });
    });
    await waitFor(() => expect(result.current.state).toBe('error'));
    expect(result.current.error?.code).toBe('mic_denied');
    expect(ws.jsonFrames().at(-1)).toEqual({ type: 'stop' });
  });

  it('maps other capture failures to mic_unavailable', async () => {
    mocks.startCapture.mockRejectedValueOnce(new Error('no device'));
    const { result } = renderSession();
    await act(async () => {
      await result.current.start();
    });
    const ws = FakeWebSocket.latest();
    act(() => ws.open());
    await act(async () => {
      ws.emit({
        type: 'ready',
        session_id: 's',
        provider: 'gemini',
        output_sample_rate: 24_000,
        thread_id: null,
      });
    });
    await waitFor(() => expect(result.current.error?.code).toBe('mic_unavailable'));
  });

  it('reports an unreachable core', async () => {
    mocks.resolveUrl.mockRejectedValueOnce(new Error('no core'));
    const { result } = renderSession();
    await act(async () => {
      await result.current.start();
    });
    expect(result.current.state).toBe('error');
    expect(result.current.error).toMatchObject({ code: 'core_unreachable', message: 'no core' });
  });

  it('ignores a second start while a session is open, and can restart after stop', async () => {
    const { result } = renderSession();
    await startAndReady(result);
    await act(async () => {
      await result.current.start();
    });
    expect(FakeWebSocket.instances).toHaveLength(1);

    act(() => result.current.stop());
    await startAndReady(result);
    expect(FakeWebSocket.instances).toHaveLength(2);
  });

  it('stops a capture that resolves after the session already ended', async () => {
    let resolveCapture: (c: unknown) => void = () => undefined;
    mocks.startCapture.mockImplementationOnce(
      () => new Promise(resolve => (resolveCapture = resolve))
    );
    const { result } = renderSession();
    await act(async () => {
      await result.current.start();
    });
    const ws = FakeWebSocket.latest();
    act(() => ws.open());
    act(() =>
      ws.emit({
        type: 'ready',
        session_id: 's',
        provider: 'gemini',
        output_sample_rate: 24_000,
        thread_id: null,
      })
    );
    act(() => result.current.stop());
    const late = { stop: vi.fn(), setMuted: vi.fn() };
    await act(async () => {
      resolveCapture(late);
    });
    expect(late.stop).toHaveBeenCalled();
  });
});
