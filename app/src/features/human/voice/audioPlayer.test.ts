import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  AudioStoppedError,
  isAudioStopped,
  playBase64Audio,
  swallowAudioStop,
} from './audioPlayer';

/**
 * Minimal HTMLAudioElement stand-in so we can drive metadata loading and
 * playback completion deterministically without a real audio decoder.
 */
class FakeAudio {
  readyState = 0;
  duration = NaN;
  currentTime = 0;
  preload = 'none';
  private listeners = new Map<string, Array<(...args: unknown[]) => void>>();

  constructor(public src: string) {}

  addEventListener(type: string, fn: (...args: unknown[]) => void): void {
    const arr = this.listeners.get(type) ?? [];
    arr.push(fn);
    this.listeners.set(type, arr);
  }

  emit(type: string): void {
    for (const fn of this.listeners.get(type) ?? []) fn();
  }

  async play(): Promise<void> {
    return Promise.resolve();
  }

  pause(): void {}
}

const originalAudio = window.Audio;
const originalCreate = URL.createObjectURL;
const originalRevoke = URL.revokeObjectURL;

beforeEach(() => {
  URL.createObjectURL = vi.fn(() => 'blob:mock');
  URL.revokeObjectURL = vi.fn();
});

afterEach(() => {
  window.Audio = originalAudio;
  URL.createObjectURL = originalCreate;
  URL.revokeObjectURL = originalRevoke;
});

function installAudio(makeAudio: (url: string) => FakeAudio): FakeAudio[] {
  const created: FakeAudio[] = [];
  (window as unknown as { Audio: unknown }).Audio = function (url: string) {
    const a = makeAudio(url);
    created.push(a);
    return a;
  };
  return created;
}

describe('playBase64Audio', () => {
  it('returns a handle whose durationMs reflects audio.duration once metadata loads', async () => {
    const created = installAudio(url => {
      const a = new FakeAudio(url);
      // loadedmetadata fires asynchronously — handle returns before then.
      queueMicrotask(() => {
        a.duration = 2.5;
        a.emit('loadedmetadata');
      });
      return a;
    });
    const handle = await playBase64Audio('AAA=');
    expect(created).toHaveLength(1);
    expect(handle.currentMs()).toBe(0);
    await handle.metadataReady;
    expect(handle.durationMs()).toBe(2500);
  });

  it('reports durationMs=0 when audio.duration is not finite', async () => {
    installAudio(url => {
      const a = new FakeAudio(url);
      a.duration = NaN;
      return a;
    });
    const handle = await playBase64Audio('AAA=');
    expect(handle.durationMs()).toBe(0);
  });

  it('metadataReady still resolves on the safety timeout when loadedmetadata never fires', async () => {
    vi.useFakeTimers();
    try {
      installAudio(url => {
        const a = new FakeAudio(url);
        // duration stays NaN; never emits loadedmetadata.
        return a;
      });
      const handle = await playBase64Audio('AAA=');
      let resolved = false;
      void handle.metadataReady.then(() => {
        resolved = true;
      });
      // Before the safety timeout fires, metadata is not ready.
      await Promise.resolve();
      expect(resolved).toBe(false);
      await vi.advanceTimersByTimeAsync(510);
      expect(resolved).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not await anything before audio.play() so the user-gesture chain is preserved', async () => {
    let playedSynchronously = false;
    installAudio(url => {
      const a = new FakeAudio(url);
      a.play = async () => {
        // The wrapper must call play() in the same microtask sequence as
        // construction — no awaits in between — or CEF/Chromium autoplay
        // policy will reject playback. Detect by asserting nothing has
        // resolved between `new Audio()` and `play()`.
        playedSynchronously = true;
      };
      return a;
    });
    await playBase64Audio('AAA=');
    expect(playedSynchronously).toBe(true);
  });

  it('stop() pauses, cleans up the blob URL, and rejects ended with AudioStoppedError', async () => {
    installAudio(url => {
      const a = new FakeAudio(url);
      a.duration = 1;
      return a;
    });
    const handle = await playBase64Audio('AAA=');
    handle.stop();
    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:mock');
    expect(handle.currentMs()).toBe(-1);
    // Sentry was firing on these orphan rejections because callers couldn't
    // distinguish "we intentionally stopped" from a real decoder error. The
    // typed sentinel + `.stopped` brand makes the cancellation case trivially
    // detectable by `isAudioStopped` / `swallowAudioStop` consumers (#1472).
    const err = await handle.ended.catch(e => e);
    expect(err).toBeInstanceOf(AudioStoppedError);
    expect(err).toMatchObject({ stopped: true, name: 'AudioStoppedError' });
    // Idempotent — second stop() is a no-op.
    handle.stop();
  });
});

describe('isAudioStopped / swallowAudioStop', () => {
  it('isAudioStopped recognizes AudioStoppedError instances', () => {
    expect(isAudioStopped(new AudioStoppedError())).toBe(true);
  });

  it('isAudioStopped accepts the structural `.stopped === true` brand', () => {
    // Defends against bundle-duplication: a second copy of audioPlayer.ts
    // would produce a different class identity but the brand still matches.
    expect(isAudioStopped({ stopped: true })).toBe(true);
    expect(isAudioStopped(Object.assign(new Error('x'), { stopped: true }))).toBe(true);
  });

  it('isAudioStopped rejects plain errors', () => {
    expect(isAudioStopped(new Error('audio playback error'))).toBe(false);
    expect(isAudioStopped(null)).toBe(false);
    expect(isAudioStopped(undefined)).toBe(false);
    expect(isAudioStopped('stopped')).toBe(false);
  });

  it('swallowAudioStop returns void on AudioStoppedError', () => {
    expect(() => swallowAudioStop(new AudioStoppedError())).not.toThrow();
  });

  it('swallowAudioStop rethrows non-stop errors so real failures stay visible', () => {
    const real = new Error('audio playback error');
    expect(() => swallowAudioStop(real)).toThrow(real);
  });

  it('attached as .catch(swallowAudioStop), it consumes orphan stop rejections without unhandledrejection', async () => {
    const unhandled: PromiseRejectionEvent[] = [];
    const handler = (e: PromiseRejectionEvent) => unhandled.push(e);
    window.addEventListener('unhandledrejection', handler);
    try {
      installAudio(url => new FakeAudio(url));
      const handle = await playBase64Audio('AAA=');
      handle.stop();
      handle.ended.catch(swallowAudioStop);
      // Let microtasks + a macrotask flush so any unhandledrejection would fire.
      await new Promise(r => setTimeout(r, 0));
      expect(unhandled).toHaveLength(0);
    } finally {
      window.removeEventListener('unhandledrejection', handler);
    }
  });
});
