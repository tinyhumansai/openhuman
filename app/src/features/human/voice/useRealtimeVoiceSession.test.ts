import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

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

vi.mock('@elevenlabs/react', () => ({
  useConversation: (props: CapturedProps) => {
    captured = props;
    return { startSession, endSession, isSpeaking: false, mode: 'listening' as const };
  },
}));

vi.mock('../../../services/api/voiceAgentApi', () => ({ fetchVoiceAgentSignedUrl: vi.fn() }));
vi.mock('../../../utils/config', () => ({ MASCOT_VOICE_ID: 'default-voice' }));

const mockFetch = vi.mocked(fetchVoiceAgentSignedUrl);

describe('useRealtimeVoiceSession', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    captured = null;
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
});
