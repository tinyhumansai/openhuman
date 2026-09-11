import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  clearAllProactiveThreadPins,
  PROACTIVE_VOICE_THREAD_ID,
  proactiveThreadPins,
} from '../../../providers/proactiveThreadPins';
import { fetchVoiceAgentSignedUrl } from '../../../services/api/voiceAgentApi';
import { useRealtimeVoiceSession } from './useRealtimeVoiceSession';

interface CapturedProps {
  onConnect: () => void;
  onDisconnect: () => void;
  onError: (message: string) => void;
}
let captured: CapturedProps | null = null;
const startSession = vi.fn();
const endSession = vi.fn();
const sendUserMessage = vi.fn();

vi.mock('@elevenlabs/react', () => ({
  useConversation: (props: CapturedProps) => {
    captured = props;
    return {
      startSession,
      endSession,
      sendUserMessage,
      isSpeaking: false,
      mode: 'listening' as const,
    };
  },
}));

// Capture the `voice_speak` subscription so a test can drive the speak-back path.
const socketHandlers: Record<string, (payload: unknown) => void> = {};
vi.mock('../../../services/socketService', () => ({
  socketService: {
    on: vi.fn((event: string, handler: (payload: unknown) => void) => {
      socketHandlers[event] = handler;
    }),
    off: vi.fn((event: string) => {
      delete socketHandlers[event];
    }),
  },
}));

vi.mock('../../../services/api/voiceAgentApi', () => ({ fetchVoiceAgentSignedUrl: vi.fn() }));
vi.mock('../../../utils/config', () => ({ MASCOT_VOICE_ID: 'default-voice' }));

const mockFetch = vi.mocked(fetchVoiceAgentSignedUrl);

describe('useRealtimeVoiceSession', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    captured = null;
    Object.keys(socketHandlers).forEach(k => delete socketHandlers[k]);
    clearAllProactiveThreadPins();
  });

  it('fetches a signed URL and opens a WebSocket session with the voice override', async () => {
    mockFetch.mockResolvedValueOnce({ signedUrl: 'wss://x', agentId: 'a1', userToken: 'tok-1' });
    const { result } = renderHook(() => useRealtimeVoiceSession({ voiceId: 'v9' }));
    expect(result.current.state).toBe('idle');

    await act(async () => {
      await result.current.start();
    });

    expect(mockFetch).toHaveBeenCalledTimes(1);
    expect(startSession).toHaveBeenCalledWith({
      signedUrl: 'wss://x',
      connectionType: 'websocket',
      userId: 'tok-1',
      customLlmExtraBody: { user: 'tok-1' },
      overrides: { tts: { voiceId: 'v9' } },
    });

    act(() => captured?.onConnect());
    expect(result.current.state).toBe('active');
  });

  // `userId` alone never reaches the Custom-LLM request the backend relay
  // serves, so the relay cannot identify the caller and rejects the turn.
  // `customLlmExtraBody` is the field that carries it there.
  it('carries the relay token in customLlmExtraBody, not only in userId', async () => {
    mockFetch.mockResolvedValueOnce({ signedUrl: 'wss://x', agentId: 'a1', userToken: 'tok-9' });
    const { result } = renderHook(() => useRealtimeVoiceSession());
    await act(async () => {
      await result.current.start();
    });
    expect(startSession).toHaveBeenCalledWith(
      expect.objectContaining({ customLlmExtraBody: { user: 'tok-9' } })
    );
  });

  it('falls back to the default mascot voice id', async () => {
    mockFetch.mockResolvedValueOnce({ signedUrl: 'wss://x', agentId: 'a1', userToken: 'tok-1' });
    const { result } = renderHook(() => useRealtimeVoiceSession());
    await act(async () => {
      await result.current.start();
    });
    expect(startSession).toHaveBeenCalledWith(
      expect.objectContaining({ overrides: { tts: { voiceId: 'default-voice' } } })
    );
  });

  it('enters the error state when the signed-URL fetch fails (no session started)', async () => {
    mockFetch.mockRejectedValueOnce(new Error('no backend session token'));
    const { result } = renderHook(() => useRealtimeVoiceSession());
    await act(async () => {
      await result.current.start();
    });
    expect(result.current.state).toBe('error');
    expect(result.current.error).toContain('no backend session token');
    expect(startSession).not.toHaveBeenCalled();
  });

  it('stop() ends the session and returns to idle', async () => {
    mockFetch.mockResolvedValueOnce({ signedUrl: 'wss://x', agentId: 'a1', userToken: 'tok-1' });
    const { result } = renderHook(() => useRealtimeVoiceSession());
    await act(async () => {
      await result.current.start();
    });
    act(() => captured?.onConnect());
    act(() => result.current.stop());
    expect(endSession).toHaveBeenCalledTimes(1);
    expect(result.current.state).toBe('idle');
  });

  it('clears the voice thread pin when a new session begins', async () => {
    // A prior session pinned proactive:voice to some thread. Starting a new
    // session must drop that pin so this session's deferred answers resolve to a
    // fresh/current thread rather than appending to the previous (now off-screen)
    // one. See resolveVisibleThreadForProactive in ChatRuntimeProvider.
    proactiveThreadPins.set(PROACTIVE_VOICE_THREAD_ID, 'previous-session-thread');
    mockFetch.mockResolvedValueOnce({ signedUrl: 'wss://x', agentId: 'a1', userToken: 'tok-1' });
    const { result } = renderHook(() => useRealtimeVoiceSession());
    await act(async () => {
      await result.current.start();
    });
    expect(proactiveThreadPins.get(PROACTIVE_VOICE_THREAD_ID)).toBeUndefined();
  });

  it('surfaces an SDK onError', () => {
    const { result } = renderHook(() => useRealtimeVoiceSession());
    act(() => captured?.onError('microphone blocked'));
    expect(result.current.state).toBe('error');
    expect(result.current.error).toBe('microphone blocked');
  });

  it('returns to idle when the SDK disconnects', async () => {
    mockFetch.mockResolvedValueOnce({ signedUrl: 'wss://x', agentId: 'a1', userToken: 'tok-1' });
    const { result } = renderHook(() => useRealtimeVoiceSession());
    await act(async () => {
      await result.current.start();
    });
    act(() => captured?.onConnect());
    expect(result.current.state).toBe('active');

    act(() => captured?.onDisconnect());
    expect(result.current.state).toBe('idle');
  });

  it('tears down a live session on unmount', async () => {
    mockFetch.mockResolvedValueOnce({ signedUrl: 'wss://x', agentId: 'a1', userToken: 'tok-1' });
    const { result, unmount } = renderHook(() => useRealtimeVoiceSession());
    await act(async () => {
      await result.current.start();
    });
    act(() => captured?.onConnect());
    unmount();
    expect(endSession).toHaveBeenCalledTimes(1);
  });

  it('does not call endSession on unmount when no session is live', () => {
    const { unmount } = renderHook(() => useRealtimeVoiceSession());
    unmount();
    expect(endSession).not.toHaveBeenCalled();
  });

  it('reads a deferred result aloud when voice_speak arrives during a live call', async () => {
    mockFetch.mockResolvedValueOnce({ signedUrl: 'wss://x', agentId: 'a1', userToken: 'tok-1' });
    const { result } = renderHook(() => useRealtimeVoiceSession());
    await act(async () => {
      await result.current.start();
    });
    act(() => captured?.onConnect()); // liveRef becomes true
    act(() => socketHandlers['voice_speak']?.({ full_response: 'Your inbox summary.' }));
    expect(sendUserMessage).toHaveBeenCalledTimes(1);
    // Wrapped in the verbatim read-back prefix so the agent reads it aloud.
    expect(sendUserMessage.mock.calls[0][0]).toContain('Your inbox summary.');
    expect(sendUserMessage.mock.calls[0][0]).toContain('Please read the following');
  });

  // Each read-back is a real agent turn, so a repeat queues behind the first and
  // pushes the call towards the provider's per-turn ceiling.
  it('reads a redelivered answer aloud only once per call', async () => {
    mockFetch.mockResolvedValue({ signedUrl: 'wss://x', agentId: 'a1', userToken: 'tok-1' });
    const { result } = renderHook(() => useRealtimeVoiceSession());
    await act(async () => {
      await result.current.start();
    });
    act(() => captured?.onConnect());
    act(() => socketHandlers['voice_speak']?.({ full_response: 'Your inbox summary.' }));
    act(() => socketHandlers['voice_speak']?.({ full_response: 'Your inbox summary.' }));
    act(() => socketHandlers['voice_speak']?.({ full_response: 'A different answer.' }));
    expect(sendUserMessage).toHaveBeenCalledTimes(2);

    // A later call is a fresh conversation: the same answer may legitimately be
    // asked for and spoken again.
    act(() => captured?.onDisconnect());
    await act(async () => {
      await result.current.start();
    });
    act(() => captured?.onConnect());
    act(() => socketHandlers['voice_speak']?.({ full_response: 'Your inbox summary.' }));
    expect(sendUserMessage).toHaveBeenCalledTimes(3);
  });

  it('ignores voice_speak when no call is live', () => {
    renderHook(() => useRealtimeVoiceSession()); // never connected → liveRef stays false
    act(() => socketHandlers['voice_speak']?.({ full_response: 'ignored' }));
    expect(sendUserMessage).not.toHaveBeenCalled();
  });

  it('ignores an empty or missing voice_speak payload', async () => {
    mockFetch.mockResolvedValueOnce({ signedUrl: 'wss://x', agentId: 'a1', userToken: 'tok-1' });
    const { result } = renderHook(() => useRealtimeVoiceSession());
    await act(async () => {
      await result.current.start();
    });
    act(() => captured?.onConnect());
    act(() => socketHandlers['voice_speak']?.({ full_response: '   ' }));
    act(() => socketHandlers['voice_speak']?.(undefined));
    expect(sendUserMessage).not.toHaveBeenCalled();
  });

  it('unsubscribes from voice_speak on unmount', () => {
    const { unmount } = renderHook(() => useRealtimeVoiceSession());
    expect(socketHandlers['voice_speak']).toBeDefined();
    unmount();
    expect(socketHandlers['voice_speak']).toBeUndefined();
  });
});
