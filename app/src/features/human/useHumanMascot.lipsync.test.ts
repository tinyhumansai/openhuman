import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { ChatEventListeners } from '../../services/chatService';
import { VISEMES } from './Mascot/visemes';
import { useHumanMascot } from './useHumanMascot';
import { playBase64Audio } from './voice/audioPlayer';
import { synthesizeSpeech } from './voice/ttsClient';

/**
 * Integration test for the audio → viseme → mouth-shape pipeline.
 *
 * Earlier, narrower tests checked `face` transitions but never asserted the
 * actual `viseme` returned by the hook while audio plays. That left a class
 * of regressions unobserved — a backend that ships viseme codes in a casing
 * the lookup table doesn't recognize, a render that doesn't re-fire as the
 * audio clock advances, frames published after `face='speaking'`, etc — all
 * looked fine to face-only tests while leaving the mouth visibly frozen on
 * REST during playback. This file exercises the full path end-to-end.
 */

vi.mock('../../services/chatService', () => ({
  subscribeChatEvents: (listeners: ChatEventListeners) => {
    capturedListeners = listeners;
    return () => {
      capturedListeners = null;
    };
  },
}));

vi.mock('./voice/ttsClient', async () => {
  const actual = await vi.importActual<typeof import('./voice/ttsClient')>('./voice/ttsClient');
  return { ...actual, synthesizeSpeech: vi.fn() };
});

vi.mock('./voice/audioPlayer', () => ({
  playBase64Audio: vi.fn(),
  // Hook's orphan `.catch(swallowAudioStop)` wiring runs in cleanup paths
  // exercised here — mirror the real helper so stop sentinels are silenced.
  swallowAudioStop: (err: unknown) => {
    if (typeof err === 'object' && err !== null && (err as { stopped?: unknown }).stopped === true)
      return;
    throw err;
  },
}));

let capturedListeners: ChatEventListeners | null = null;

interface FakePlayback {
  handle: {
    currentMs: () => number;
    durationMs: () => number;
    metadataReady: Promise<void>;
    stop: () => void;
    ended: Promise<void>;
  };
  setMs(ms: number): void;
  finish(): void;
}

function makePlayback(durationMs: number): FakePlayback {
  let ms = 0;
  let stopped = false;
  let resolveEnded!: () => void;
  const ended = new Promise<void>(res => {
    resolveEnded = res;
  });
  return {
    handle: {
      currentMs: () => (stopped ? -1 : ms),
      durationMs: () => durationMs,
      metadataReady: Promise.resolve(),
      stop: () => {
        stopped = true;
      },
      ended,
    },
    setMs(next: number) {
      ms = next;
    },
    finish() {
      stopped = true;
      resolveEnded();
    },
  };
}

/**
 * Drive the hook's RAF-based render loop deterministically. The hook calls
 * `requestAnimationFrame` on every speaking frame; without firing it the
 * `viseme` value never refreshes between renders.
 */
let rafQueue: FrameRequestCallback[] = [];
const originalRaf = window.requestAnimationFrame;
const originalCancel = window.cancelAnimationFrame;

beforeEach(() => {
  capturedListeners = null;
  rafQueue = [];
  (synthesizeSpeech as ReturnType<typeof vi.fn>).mockReset();
  (playBase64Audio as ReturnType<typeof vi.fn>).mockReset();
  window.requestAnimationFrame = ((cb: FrameRequestCallback) => {
    rafQueue.push(cb);
    return rafQueue.length;
  }) as typeof window.requestAnimationFrame;
  window.cancelAnimationFrame = (() => {}) as typeof window.cancelAnimationFrame;
});

afterEach(() => {
  window.requestAnimationFrame = originalRaf;
  window.cancelAnimationFrame = originalCancel;
});

function tickRaf() {
  const queue = rafQueue;
  rafQueue = [];
  for (const cb of queue) cb(performance.now());
}

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

describe('useHumanMascot — audio-driven lipsync end-to-end', () => {
  it('mouth opens (non-REST) once playback starts and visemes have known codes', async () => {
    const fake = makePlayback(1000);
    (synthesizeSpeech as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      audio_base64: 'AAA=',
      audio_mime: 'audio/mpeg',
      visemes: [
        { viseme: 'aa', start_ms: 0, end_ms: 200 }, // wide open vowel
        { viseme: 'PP', start_ms: 200, end_ms: 400 }, // closed bilabial
      ],
    });
    (playBase64Audio as ReturnType<typeof vi.fn>).mockResolvedValueOnce(fake.handle);

    const { result } = renderHook(() => useHumanMascot({ speakReplies: true }));

    // Drive the full async chain: onDone → synthesizeSpeech → playBase64Audio
    // → setFace('speaking'). Then fire a RAF tick so the hook re-renders with
    // playbackRef.current populated.
    await act(async () => {
      capturedListeners?.onDone?.(fakeDone('hello'));
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(result.current.face).toBe('speaking');

    // ms=0 → frame[0] = 'aa' = wide-open A.
    act(() => {
      fake.setMs(50);
      tickRaf();
    });
    expect(result.current.viseme).toEqual(VISEMES.A);
    expect(result.current.viseme).not.toEqual(VISEMES.REST);

    // ms=300 → frame[1] = 'PP' = closed M.
    act(() => {
      fake.setMs(300);
      tickRaf();
    });
    expect(result.current.viseme).toEqual(VISEMES.M);
  });

  it('mouth opens even when backend ships visemes in lowercase / aliased codes', async () => {
    const fake = makePlayback(1000);
    (synthesizeSpeech as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      audio_base64: 'AAA=',
      audio_mime: 'audio/mpeg',
      // Real-world regression: a backend might ship `pp` lowercase, or bare
      // letter codes like `a` / `o` instead of `aa` / `O`. The lookup must
      // accept both vocabularies — otherwise every frame maps to REST and
      // the mouth visibly freezes on the rest-smile path while audio plays.
      visemes: [
        { viseme: 'a', start_ms: 0, end_ms: 200 },
        { viseme: 'pp', start_ms: 200, end_ms: 400 },
        { viseme: 'O', start_ms: 400, end_ms: 600 },
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

    act(() => {
      fake.setMs(50);
      tickRaf();
    });
    expect(result.current.viseme).toEqual(VISEMES.A);

    act(() => {
      fake.setMs(250);
      tickRaf();
    });
    expect(result.current.viseme).toEqual(VISEMES.M);

    act(() => {
      fake.setMs(500);
      tickRaf();
    });
    expect(result.current.viseme).toEqual(VISEMES.O);
  });

  it('mouth still animates when backend ships unknown viseme codes (procedural fallback)', async () => {
    const fake = makePlayback(1000);
    (synthesizeSpeech as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      audio_base64: 'AAA=',
      audio_mime: 'audio/mpeg',
      // All codes unknown to oculusVisemeToShape — without the all-REST
      // detector the mouth would freeze, but the hook should fall through
      // to procedural visemes derived from the text.
      visemes: [
        { viseme: '???', start_ms: 0, end_ms: 200 },
        { viseme: 'unknown_code', start_ms: 200, end_ms: 400 },
      ],
    });
    (playBase64Audio as ReturnType<typeof vi.fn>).mockResolvedValueOnce(fake.handle);

    const { result } = renderHook(() => useHumanMascot({ speakReplies: true }));
    await act(async () => {
      capturedListeners?.onDone?.(fakeDone('hello world'));
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    // Sample several timestamps across the clip; at least one must produce
    // a non-REST shape, otherwise the mouth would visibly freeze.
    const sampled = new Set<string>();
    for (const ms of [10, 100, 250, 400, 600, 800]) {
      act(() => {
        fake.setMs(ms);
        tickRaf();
      });
      sampled.add(JSON.stringify(result.current.viseme));
    }
    expect(sampled.has(JSON.stringify(VISEMES.REST))).toBe(false);
    // Multiple distinct shapes proves the mouth is actually animating, not
    // just stuck on a single non-REST frame.
    expect(sampled.size).toBeGreaterThanOrEqual(2);
  });

  it('mouth animates with no visemes and no alignment (full procedural path)', async () => {
    const fake = makePlayback(1000);
    (synthesizeSpeech as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      audio_base64: 'AAA=',
      audio_mime: 'audio/mpeg',
      visemes: [],
      // no alignment either — pure last-resort fallback from text length.
    });
    (playBase64Audio as ReturnType<typeof vi.fn>).mockResolvedValueOnce(fake.handle);

    const { result } = renderHook(() => useHumanMascot({ speakReplies: true }));
    await act(async () => {
      capturedListeners?.onDone?.(fakeDone('the mascot is speaking right now'));
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(result.current.face).toBe('speaking');

    const sampled = new Set<string>();
    for (const ms of [20, 150, 320, 500, 720, 900]) {
      act(() => {
        fake.setMs(ms);
        tickRaf();
      });
      sampled.add(JSON.stringify(result.current.viseme));
    }
    expect(sampled.has(JSON.stringify(VISEMES.REST))).toBe(false);
    expect(sampled.size).toBeGreaterThanOrEqual(2);
  });

  it('mouth returns to a non-speaking shape once playback ends', async () => {
    const fake = makePlayback(500);
    (synthesizeSpeech as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      audio_base64: 'AAA=',
      audio_mime: 'audio/mpeg',
      visemes: [{ viseme: 'aa', start_ms: 0, end_ms: 500 }],
    });
    (playBase64Audio as ReturnType<typeof vi.fn>).mockResolvedValueOnce(fake.handle);

    const { result } = renderHook(() => useHumanMascot({ speakReplies: true }));
    await act(async () => {
      capturedListeners?.onDone?.(fakeDone('hi'));
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    act(() => {
      fake.setMs(100);
      tickRaf();
    });
    expect(result.current.viseme).toEqual(VISEMES.A);

    await act(async () => {
      fake.finish();
      await Promise.resolve();
      await Promise.resolve();
    });
    // Face leaves speaking once audio ends — the rest-mouth is rendered by
    // Ghosty rather than via `viseme`, so we just assert the lifecycle moved
    // off speaking.
    expect(result.current.face).not.toBe('speaking');
  });
});
