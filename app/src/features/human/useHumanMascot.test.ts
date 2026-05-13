import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { ChatEventListeners } from '../../services/chatService';
import { VISEMES } from './Mascot/visemes';
import { ACK_FACE_HOLD_MS, pickViseme, useHumanMascot } from './useHumanMascot';
import { type PlaybackHandle, playBase64Audio } from './voice/audioPlayer';
import { synthesizeSpeech } from './voice/ttsClient';

vi.mock('../../services/chatService', () => ({
  subscribeChatEvents: (listeners: ChatEventListeners) => {
    capturedListeners = listeners;
    return () => {
      capturedListeners = null;
    };
  },
}));

const proceduralVisemesMock = vi.fn(
  (text: string, durationMs: number): { viseme: string; start_ms: number; end_ms: number }[] => {
    if (!text) return [];
    return [{ viseme: 'aa', start_ms: 0, end_ms: durationMs || 100 }];
  }
);

vi.mock('./voice/ttsClient', () => ({
  synthesizeSpeech: vi.fn(),
  visemesFromAlignment: (alignment: { char: string; start_ms: number; end_ms: number }[]) =>
    alignment.map(a => ({ viseme: 'aa', start_ms: a.start_ms, end_ms: a.end_ms })),
  proceduralVisemes: (text: string, durationMs: number) => proceduralVisemesMock(text, durationMs),
}));

class FakeAudioStoppedError extends Error {
  readonly stopped = true;
  constructor() {
    super('stopped');
    this.name = 'AudioStoppedError';
  }
}

vi.mock('./voice/audioPlayer', () => ({
  playBase64Audio: vi.fn(),
  // Mirror the real helper so the hook's orphan `.catch(swallowAudioStop)`
  // wiring actually executes — otherwise stop sentinels would slip through
  // as unhandledrejections under test and the regression coverage is moot.
  swallowAudioStop: (err: unknown) => {
    if (typeof err === 'object' && err !== null && (err as { stopped?: unknown }).stopped === true)
      return;
    throw err;
  },
}));

function makeFakePlayback(durationMs = 100) {
  let stopped = false;
  let resolveEnded!: () => void;
  let rejectEnded!: (e: Error) => void;
  const ended = new Promise<void>((res, rej) => {
    resolveEnded = res;
    rejectEnded = rej;
  });
  return {
    handle: {
      currentMs: () => (stopped ? -1 : 0),
      durationMs: () => durationMs,
      metadataReady: Promise.resolve(),
      stop: () => {
        stopped = true;
        rejectEnded(new FakeAudioStoppedError());
      },
      ended,
    },
    finishNaturally: () => {
      stopped = true;
      resolveEnded();
    },
    durationMs,
  };
}

let capturedListeners: ChatEventListeners | null = null;

describe('pickViseme', () => {
  it('maps vowels to their viseme', () => {
    expect(pickViseme('a')).toBe(VISEMES.A);
    expect(pickViseme('e')).toBe(VISEMES.E);
    expect(pickViseme('i')).toBe(VISEMES.I);
    expect(pickViseme('o')).toBe(VISEMES.O);
    expect(pickViseme('u')).toBe(VISEMES.U);
  });

  it('maps labials to M', () => {
    expect(pickViseme('m')).toBe(VISEMES.M);
    expect(pickViseme('b')).toBe(VISEMES.M);
    expect(pickViseme('p')).toBe(VISEMES.M);
  });

  it('maps fricatives to F', () => {
    expect(pickViseme('f')).toBe(VISEMES.F);
    expect(pickViseme('v')).toBe(VISEMES.F);
  });

  it('uses the trailing letter of multi-char deltas', () => {
    expect(pickViseme('hello')).toBe(VISEMES.O);
    expect(pickViseme('world')).toBe(VISEMES.E); // d → fallback
  });

  it('ignores punctuation when picking the trailing letter', () => {
    expect(pickViseme('Hi!')).toBe(VISEMES.I);
    expect(pickViseme('...')).toBe(VISEMES.E); // no letters → fallback
  });

  it('falls back to E for unmapped consonants', () => {
    expect(pickViseme('z')).toBe(VISEMES.E);
    expect(pickViseme('')).toBe(VISEMES.E);
  });
});

describe('useHumanMascot state machine', () => {
  beforeEach(() => {
    capturedListeners = null;
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  function fakeEvent<T>(extra: T): T & { thread_id: string; request_id: string } {
    return { thread_id: 't', request_id: 'r', ...extra };
  }

  it('starts idle', () => {
    const { result } = renderHook(() => useHumanMascot());
    expect(result.current.face).toBe('idle');
  });

  it('moves to thinking on inference_start', () => {
    const { result } = renderHook(() => useHumanMascot());
    act(() => {
      capturedListeners?.onInferenceStart?.(fakeEvent({}));
    });
    expect(result.current.face).toBe('thinking');
  });

  it('moves to confused on tool_call', () => {
    const { result } = renderHook(() => useHumanMascot());
    act(() => {
      capturedListeners?.onInferenceStart?.(fakeEvent({}));
      capturedListeners?.onToolCall?.(
        fakeEvent({ tool_name: 'search', skill_id: 's', args: {}, round: 1 })
      );
    });
    expect(result.current.face).toBe('confused');
  });

  it('moves to confused on iteration_start beyond round 1', () => {
    const { result } = renderHook(() => useHumanMascot());
    act(() => {
      capturedListeners?.onInferenceStart?.(fakeEvent({}));
      capturedListeners?.onIterationStart?.(fakeEvent({ round: 2, message: '' }));
    });
    expect(result.current.face).toBe('confused');
  });

  it('does not flip to confused on iteration_start round 1', () => {
    const { result } = renderHook(() => useHumanMascot());
    act(() => {
      capturedListeners?.onInferenceStart?.(fakeEvent({}));
      capturedListeners?.onIterationStart?.(fakeEvent({ round: 1, message: '' }));
    });
    expect(result.current.face).toBe('thinking');
  });

  it('moves to concerned on failed tool result', () => {
    const { result } = renderHook(() => useHumanMascot());
    act(() => {
      capturedListeners?.onToolResult?.(
        fakeEvent({ tool_name: 'search', skill_id: 's', output: 'oops', success: false, round: 1 })
      );
    });
    expect(result.current.face).toBe('concerned');
  });

  it('moves to speaking on text_delta', () => {
    const { result } = renderHook(() => useHumanMascot());
    act(() => {
      capturedListeners?.onTextDelta?.(fakeEvent({ round: 1, delta: 'hello' }));
    });
    expect(result.current.face).toBe('speaking');
  });

  it('holds happy briefly on chat_done without speakReplies, then idles', () => {
    const { result } = renderHook(() => useHumanMascot({ speakReplies: false }));
    act(() => {
      capturedListeners?.onDone?.(
        fakeEvent({
          full_response: 'hello',
          rounds_used: 1,
          total_input_tokens: 1,
          total_output_tokens: 1,
        })
      );
    });
    expect(result.current.face).toBe('happy');
    act(() => {
      vi.advanceTimersByTime(ACK_FACE_HOLD_MS + 1);
    });
    expect(result.current.face).toBe('idle');
  });

  it('holds concerned briefly on chat_error, then idles', () => {
    const { result } = renderHook(() => useHumanMascot());
    act(() => {
      capturedListeners?.onError?.(
        fakeEvent({ message: 'boom', error_type: 'inference', round: 1 })
      );
    });
    expect(result.current.face).toBe('concerned');
    act(() => {
      vi.advanceTimersByTime(ACK_FACE_HOLD_MS + 1);
    });
    expect(result.current.face).toBe('idle');
  });

  it('listening option overrides non-speaking faces', () => {
    const { result, rerender } = renderHook(
      ({ listening }: { listening: boolean }) => useHumanMascot({ listening }),
      { initialProps: { listening: false } }
    );
    expect(result.current.face).toBe('idle');
    rerender({ listening: true });
    expect(result.current.face).toBe('listening');
  });

  it('clears the ack timer when a new turn starts before the hold finishes', () => {
    const { result } = renderHook(() => useHumanMascot({ speakReplies: false }));
    act(() => {
      capturedListeners?.onDone?.(
        fakeEvent({
          full_response: 'hi',
          rounds_used: 1,
          total_input_tokens: 1,
          total_output_tokens: 1,
        })
      );
    });
    expect(result.current.face).toBe('happy');
    act(() => {
      capturedListeners?.onInferenceStart?.(fakeEvent({}));
    });
    expect(result.current.face).toBe('thinking');
    // Advancing past the original hold must NOT flip back to idle since the
    // timer was cleared by the new turn.
    act(() => {
      vi.advanceTimersByTime(ACK_FACE_HOLD_MS + 1);
    });
    expect(result.current.face).toBe('thinking');
  });

  it('successful tool result returns the face to thinking', () => {
    const { result } = renderHook(() => useHumanMascot());
    act(() => {
      capturedListeners?.onToolResult?.(
        fakeEvent({ tool_name: 'search', skill_id: 's', output: 'ok', success: true, round: 1 })
      );
    });
    expect(result.current.face).toBe('thinking');
  });

  it('listening does not override speaking', () => {
    const { result, rerender } = renderHook(
      ({ listening }: { listening: boolean }) => useHumanMascot({ listening }),
      { initialProps: { listening: false } }
    );
    act(() => {
      capturedListeners?.onTextDelta?.(fakeEvent({ round: 1, delta: 'hi' }));
    });
    rerender({ listening: true });
    expect(result.current.face).toBe('speaking');
  });
});

describe('useHumanMascot TTS playback', () => {
  beforeEach(() => {
    capturedListeners = null;
    vi.useFakeTimers();
    (synthesizeSpeech as ReturnType<typeof vi.fn>).mockReset();
    (playBase64Audio as ReturnType<typeof vi.fn>).mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  function fakeDone(text: string) {
    return {
      thread_id: 't',
      request_id: 'r',
      full_response: text,
      rounds_used: 1,
      total_input_tokens: 1,
      total_output_tokens: 1,
    };
  }

  it('runs a full TTS playback flow: thinking → speaking → happy → idle', async () => {
    const fake = makeFakePlayback();
    (synthesizeSpeech as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      audio_base64: 'AAA=',
      audio_mime: 'audio/mpeg',
      visemes: [{ viseme: 'aa', start_ms: 0, end_ms: 100 }],
    });
    (playBase64Audio as ReturnType<typeof vi.fn>).mockResolvedValueOnce(fake.handle);

    const { result } = renderHook(() => useHumanMascot({ speakReplies: true }));
    await act(async () => {
      capturedListeners?.onDone?.(fakeDone('hello'));
      // Let synthesizeSpeech and playBase64Audio resolve.
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(result.current.face).toBe('speaking');

    await act(async () => {
      fake.finishNaturally();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(result.current.face).toBe('happy');

    act(() => {
      vi.advanceTimersByTime(ACK_FACE_HOLD_MS + 1);
    });
    expect(result.current.face).toBe('idle');
  });

  it('falls back to alignment-derived visemes when backend ships no cues', async () => {
    const fake = makeFakePlayback();
    (synthesizeSpeech as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      audio_base64: 'AAA=',
      audio_mime: 'audio/mpeg',
      visemes: [],
      alignment: [{ char: 'h', start_ms: 0, end_ms: 50 }],
    });
    (playBase64Audio as ReturnType<typeof vi.fn>).mockResolvedValueOnce(fake.handle);

    const { result } = renderHook(() => useHumanMascot({ speakReplies: true }));
    await act(async () => {
      capturedListeners?.onDone?.(fakeDone('hi'));
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(result.current.face).toBe('speaking');
    await act(async () => {
      fake.finishNaturally();
      await Promise.resolve();
      await Promise.resolve();
    });
  });

  it('falls back to procedural visemes when backend ships neither cues nor alignment', async () => {
    const fake = makeFakePlayback(2000);
    proceduralVisemesMock.mockClear();
    (synthesizeSpeech as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      audio_base64: 'AAA=',
      audio_mime: 'audio/mpeg',
      visemes: [],
    });
    (playBase64Audio as ReturnType<typeof vi.fn>).mockResolvedValueOnce(fake.handle);

    const { result } = renderHook(() => useHumanMascot({ speakReplies: true }));
    await act(async () => {
      capturedListeners?.onDone?.(fakeDone('hello there'));
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(result.current.face).toBe('speaking');
    expect(proceduralVisemesMock).toHaveBeenCalledWith('hello there', 2000);

    await act(async () => {
      fake.finishNaturally();
      await Promise.resolve();
      await Promise.resolve();
    });
  });

  it('falls back to procedural visemes when backend frames all map to REST', async () => {
    const fake = makeFakePlayback(2000);
    proceduralVisemesMock.mockClear();
    (synthesizeSpeech as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      audio_base64: 'AAA=',
      audio_mime: 'audio/mpeg',
      // `???` and `unknown` are not in the viseme table — every frame would
      // map to REST and the mouth would freeze. The hook should detect this
      // and fall through to the procedural path.
      visemes: [
        { viseme: '???', start_ms: 0, end_ms: 100 },
        { viseme: 'unknown', start_ms: 100, end_ms: 200 },
      ],
    });
    (playBase64Audio as ReturnType<typeof vi.fn>).mockResolvedValueOnce(fake.handle);

    const { result } = renderHook(() => useHumanMascot({ speakReplies: true }));
    await act(async () => {
      capturedListeners?.onDone?.(fakeDone('hi'));
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(result.current.face).toBe('speaking');
    expect(proceduralVisemesMock).toHaveBeenCalledWith('hi', 2000);

    await act(async () => {
      fake.finishNaturally();
      await Promise.resolve();
      await Promise.resolve();
    });
  });

  it('does not surface an unhandledrejection when a newer turn cancels in-flight playback (#1472)', async () => {
    // Two back-to-back turns: the first reaches the `await playBase64Audio`
    // point and then a second onDone bumps the playback seq. When the first
    // play() finally resolves, the hook takes the `!isStillCurrent()` branch
    // and calls `handle.stop()` + early-returns. Before the fix, that left
    // the resulting `handle.ended` rejection un-attached → unhandledrejection
    // → Sentry. The fix attaches `.catch(swallowAudioStop)` at each such site.
    vi.useRealTimers();
    const fake1 = makeFakePlayback();
    const fake2 = makeFakePlayback();
    let resolveFirstPlay!: (h: PlaybackHandle) => void;
    const firstPlay = new Promise<PlaybackHandle>(r => {
      resolveFirstPlay = r;
    });
    (synthesizeSpeech as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce({
        audio_base64: 'AAA=',
        audio_mime: 'audio/mpeg',
        visemes: [{ viseme: 'aa', start_ms: 0, end_ms: 100 }],
      })
      .mockResolvedValueOnce({
        audio_base64: 'BBB=',
        audio_mime: 'audio/mpeg',
        visemes: [{ viseme: 'aa', start_ms: 0, end_ms: 100 }],
      });
    (playBase64Audio as ReturnType<typeof vi.fn>)
      .mockImplementationOnce(() => firstPlay)
      .mockResolvedValueOnce(fake2.handle);

    const unhandled: PromiseRejectionEvent[] = [];
    const handler = (e: PromiseRejectionEvent) => unhandled.push(e);
    window.addEventListener('unhandledrejection', handler);
    try {
      renderHook(() => useHumanMascot({ speakReplies: true }));
      // Turn 1 enters startTtsPlayback and blocks on playBase64Audio.
      await act(async () => {
        capturedListeners?.onDone?.(fakeDone('first'));
        await Promise.resolve();
        await Promise.resolve();
      });
      // Turn 2 fires, bumps playbackSeqRef, awaits its own (resolved) play.
      await act(async () => {
        capturedListeners?.onDone?.(fakeDone('second'));
        await Promise.resolve();
        await Promise.resolve();
        await Promise.resolve();
      });
      // Now resolve turn-1's play: its handle is stale → hook stops + bails.
      await act(async () => {
        resolveFirstPlay(fake1.handle as unknown as PlaybackHandle);
        await Promise.resolve();
        await Promise.resolve();
        await Promise.resolve();
        // Macrotask hop so jsdom can dispatch any pending unhandledrejection.
        await new Promise(r => setTimeout(r, 0));
      });
      expect(unhandled).toHaveLength(0);
    } finally {
      window.removeEventListener('unhandledrejection', handler);
      vi.useFakeTimers();
    }
  });

  it('shows concerned (not happy) when synthesizeSpeech rejects', async () => {
    (synthesizeSpeech as ReturnType<typeof vi.fn>).mockRejectedValueOnce(new Error('voice down'));

    const { result } = renderHook(() => useHumanMascot({ speakReplies: true }));
    await act(async () => {
      capturedListeners?.onDone?.(fakeDone('hello'));
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(result.current.face).toBe('concerned');
    act(() => {
      vi.advanceTimersByTime(ACK_FACE_HOLD_MS + 1);
    });
    expect(result.current.face).toBe('idle');
  });
});
