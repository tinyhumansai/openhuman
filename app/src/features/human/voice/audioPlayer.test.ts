import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { playBase64Audio } from './audioPlayer';

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
  it('waits for loadedmetadata and exposes audio.duration in ms', async () => {
    const created = installAudio(url => {
      const a = new FakeAudio(url);
      // Defer metadata so the wrapper's readyState branch must wait.
      queueMicrotask(() => {
        a.readyState = 1;
        a.duration = 2.5; // 2500ms
        a.emit('loadedmetadata');
      });
      return a;
    });
    const handle = await playBase64Audio('AAA=');
    expect(created).toHaveLength(1);
    expect(handle.durationMs).toBe(2500);
    expect(handle.currentMs()).toBe(0);
  });

  it('falls back to durationMs=0 when audio.duration is not finite', async () => {
    installAudio(url => {
      const a = new FakeAudio(url);
      // Skip the wait branch entirely so we exercise the !isFinite path.
      a.readyState = 4;
      a.duration = NaN;
      return a;
    });
    const handle = await playBase64Audio('AAA=');
    expect(handle.durationMs).toBe(0);
  });

  it('does not block forever when loadedmetadata never fires (timeout race)', async () => {
    vi.useFakeTimers();
    try {
      installAudio(url => {
        const a = new FakeAudio(url);
        a.readyState = 0;
        // duration stays NaN; never emits loadedmetadata.
        return a;
      });
      const promise = playBase64Audio('AAA=');
      // Advance past the 250ms safety timeout in audioPlayer.ts.
      await vi.advanceTimersByTimeAsync(260);
      const handle = await promise;
      expect(handle.durationMs).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('stop() pauses, cleans up the blob URL, and rejects ended', async () => {
    installAudio(url => {
      const a = new FakeAudio(url);
      a.readyState = 1;
      a.duration = 1;
      return a;
    });
    const handle = await playBase64Audio('AAA=');
    handle.stop();
    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:mock');
    expect(handle.currentMs()).toBe(-1);
    await expect(handle.ended).rejects.toThrow('stopped');
    // Idempotent — second stop() is a no-op.
    handle.stop();
  });
});
