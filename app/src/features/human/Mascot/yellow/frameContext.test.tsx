import { act, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { FrameProvider, useCurrentFrame, useVideoConfig } from './frameContext';

interface RAFCallback {
  (now: number): void;
}

function mockRequestAnimationFrame() {
  const callbacks = new Map<number, RAFCallback>();
  let nextId = 1;

  const raf = vi.fn((cb: RAFCallback): number => {
    const id = nextId++;
    callbacks.set(id, cb);
    return id;
  });
  const caf = vi.fn((id: number) => {
    callbacks.delete(id);
  });

  const tickTo = (now: number) => {
    const pending = Array.from(callbacks.entries());
    callbacks.clear();
    for (const [, cb] of pending) cb(now);
  };

  return { raf, caf, tickTo };
}

describe('frameContext', () => {
  let original: {
    raf: typeof window.requestAnimationFrame;
    caf: typeof window.cancelAnimationFrame;
  };
  let mock: ReturnType<typeof mockRequestAnimationFrame>;

  beforeEach(() => {
    mock = mockRequestAnimationFrame();
    original = { raf: window.requestAnimationFrame, caf: window.cancelAnimationFrame };
    window.requestAnimationFrame = mock.raf as unknown as typeof window.requestAnimationFrame;
    window.cancelAnimationFrame = mock.caf as unknown as typeof window.cancelAnimationFrame;
  });

  afterEach(() => {
    window.requestAnimationFrame = original.raf;
    window.cancelAnimationFrame = original.caf;
  });

  it('exposes the configured video config to consumers', () => {
    let captured: ReturnType<typeof useVideoConfig> | null = null;
    const Probe = () => {
      captured = useVideoConfig();
      return null;
    };
    render(
      <FrameProvider fps={30} width={500} height={500} durationInFrames={180}>
        <Probe />
      </FrameProvider>
    );
    expect(captured).toEqual({ fps: 30, width: 500, height: 500, durationInFrames: 180 });
  });

  it('starts at frame 0 and advances based on elapsed time', () => {
    let frame = -1;
    const Probe = () => {
      frame = useCurrentFrame();
      return null;
    };
    render(
      <FrameProvider fps={30} width={100} height={100} durationInFrames={180}>
        <Probe />
      </FrameProvider>
    );
    // First render before any rAF tick.
    expect(frame).toBe(0);
    // Advance 0.5s — at 30fps this is frame 15.
    act(() => mock.tickTo(0));
    act(() => mock.tickTo(500));
    expect(frame).toBe(15);
    // Advance another 0.5s — frame 30.
    act(() => mock.tickTo(1000));
    expect(frame).toBe(30);
  });

  it('loops back to frame 0 after durationInFrames', () => {
    let frame = -1;
    const Probe = () => {
      frame = useCurrentFrame();
      return null;
    };
    render(
      <FrameProvider fps={30} width={100} height={100} durationInFrames={60}>
        <Probe />
      </FrameProvider>
    );
    act(() => mock.tickTo(0));
    // 2 seconds at 30fps = 60 frames → wraps to 0.
    act(() => mock.tickTo(2000));
    expect(frame).toBe(0);
    // 2.5s = 75 frames → 75 % 60 = 15.
    act(() => mock.tickTo(2500));
    expect(frame).toBe(15);
  });

  it('throws when useVideoConfig is used outside FrameProvider', () => {
    const Probe = () => {
      useVideoConfig();
      return null;
    };
    // Suppress React's error logging for this throw-on-render case.
    const errSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    expect(() => render(<Probe />)).toThrow(/useVideoConfig/);
    errSpy.mockRestore();
  });
});
