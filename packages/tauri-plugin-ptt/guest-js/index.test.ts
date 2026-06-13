/**
 * Unit tests for tauri-plugin-ptt JS bindings.
 *
 * Verifies that each exported function calls the correct Tauri command name
 * with the correct argument structure, and that event subscriptions call
 * `listen` with the expected event name.
 */
import { describe, expect, it, vi, beforeEach } from 'vitest';

// Mock @tauri-apps/api/core and @tauri-apps/api/event before importing the module.
const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: unknown) => mockInvoke(cmd, args),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: (event: string, cb: unknown) => mockListen(event, cb),
}));

import {
  cancelSpeech,
  listVoices,
  onError,
  onTranscriptFinal,
  onTranscriptPartial,
  onTtsEnded,
  onTtsStarted,
  speak,
  startListening,
  stopListening,
} from './index';

beforeEach(() => {
  mockInvoke.mockReset();
  mockListen.mockReset();
  mockInvoke.mockResolvedValue(undefined);
  mockListen.mockResolvedValue(vi.fn());
});

// ── Commands ─────────────────────────────────────────────────────────────────

describe('startListening', () => {
  it('invokes plugin:ptt|start_listening', async () => {
    await startListening();
    expect(mockInvoke.mock.calls[0][0]).toBe('plugin:ptt|start_listening');
  });
});

describe('stopListening', () => {
  it('invokes plugin:ptt|stop_listening', async () => {
    mockInvoke.mockResolvedValueOnce({ text: 'hello', isFinal: true });
    const result = await stopListening();
    expect(mockInvoke.mock.calls[0][0]).toBe('plugin:ptt|stop_listening');
    expect(result).toEqual({ text: 'hello', isFinal: true });
  });
});

describe('speak', () => {
  it('invokes plugin:ptt|speak with text and null opts', async () => {
    await speak('Hello world');
    expect(mockInvoke).toHaveBeenCalledWith('plugin:ptt|speak', {
      text: 'Hello world',
      voiceId: null,
      rate: null,
    });
  });

  it('passes voiceId and rate when provided', async () => {
    await speak('Hi', { voiceId: 'com.apple.voice.compact.en-US.Samantha', rate: 1.2 });
    expect(mockInvoke).toHaveBeenCalledWith('plugin:ptt|speak', {
      text: 'Hi',
      voiceId: 'com.apple.voice.compact.en-US.Samantha',
      rate: 1.2,
    });
  });
});

describe('cancelSpeech', () => {
  it('invokes plugin:ptt|cancel_speech', async () => {
    await cancelSpeech();
    expect(mockInvoke.mock.calls[0][0]).toBe('plugin:ptt|cancel_speech');
  });
});

describe('listVoices', () => {
  it('invokes plugin:ptt|list_voices and returns voice list', async () => {
    const voices = [{ id: 'v1', name: 'Samantha', lang: 'en-US' }];
    mockInvoke.mockResolvedValueOnce(voices);
    const result = await listVoices();
    expect(mockInvoke.mock.calls[0][0]).toBe('plugin:ptt|list_voices');
    expect(result).toEqual(voices);
  });
});

// ── Event subscriptions ──────────────────────────────────────────────────────

describe('onTranscriptPartial', () => {
  it('calls listen with ptt://transcript-partial', async () => {
    const cb = vi.fn();
    await onTranscriptPartial(cb);
    expect(mockListen).toHaveBeenCalledWith('ptt://transcript-partial', expect.any(Function));
  });

  it('delivers text from the event payload', async () => {
    let capturedHandler: ((e: { payload: { text: string } }) => void) | undefined;
    mockListen.mockImplementation((_event: string, handler: (e: { payload: { text: string } }) => void) => {
      capturedHandler = handler;
      return Promise.resolve(vi.fn());
    });

    const cb = vi.fn();
    await onTranscriptPartial(cb);

    capturedHandler?.({ payload: { text: 'partial text' } });
    expect(cb).toHaveBeenCalledWith('partial text');
  });
});

describe('onTranscriptFinal', () => {
  it('calls listen with ptt://transcript-final', async () => {
    await onTranscriptFinal(vi.fn());
    expect(mockListen).toHaveBeenCalledWith('ptt://transcript-final', expect.any(Function));
  });
});

describe('onTtsStarted', () => {
  it('calls listen with ptt://tts-started and delivers utteranceId', async () => {
    let capturedHandler: ((e: { payload: { utteranceId: string } }) => void) | undefined;
    mockListen.mockImplementation((_event: string, handler: (e: { payload: { utteranceId: string } }) => void) => {
      capturedHandler = handler;
      return Promise.resolve(vi.fn());
    });

    const cb = vi.fn();
    await onTtsStarted(cb);

    expect(mockListen).toHaveBeenCalledWith('ptt://tts-started', expect.any(Function));
    capturedHandler?.({ payload: { utteranceId: 'uid-123' } });
    expect(cb).toHaveBeenCalledWith('uid-123');
  });
});

describe('onTtsEnded', () => {
  it('calls listen with ptt://tts-ended and delivers utteranceId + finished', async () => {
    let capturedHandler: ((e: { payload: { utteranceId: string; finished: boolean } }) => void) | undefined;
    mockListen.mockImplementation((_event: string, handler: (e: { payload: { utteranceId: string; finished: boolean } }) => void) => {
      capturedHandler = handler;
      return Promise.resolve(vi.fn());
    });

    const cb = vi.fn();
    await onTtsEnded(cb);

    expect(mockListen).toHaveBeenCalledWith('ptt://tts-ended', expect.any(Function));
    capturedHandler?.({ payload: { utteranceId: 'uid-456', finished: false } });
    expect(cb).toHaveBeenCalledWith('uid-456', false);
  });
});

describe('onError', () => {
  it('calls listen with ptt://error', async () => {
    await onError(vi.fn());
    expect(mockListen).toHaveBeenCalledWith('ptt://error', expect.any(Function));
  });

  it('delivers error payload', async () => {
    let capturedHandler: ((e: { payload: { code: string; message: string } }) => void) | undefined;
    mockListen.mockImplementation((_event: string, handler: (e: { payload: { code: string; message: string } }) => void) => {
      capturedHandler = handler;
      return Promise.resolve(vi.fn());
    });

    const cb = vi.fn();
    await onError(cb);

    capturedHandler?.({ payload: { code: 'interrupted', message: 'call came in' } });
    expect(cb).toHaveBeenCalledWith({ code: 'interrupted', message: 'call came in' });
  });
});
