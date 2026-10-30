import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useMascotClock } from './useMascotClock';

let now = 0;
let hidden = false;
let reduced = false;
let nextFrame = 0;
const frames = new Map<number, FrameRequestCallback>();
let motion: EventTarget;

function frame(afterMs: number) {
  now += afterMs;
  const callbacks = [...frames.values()];
  frames.clear();
  act(() => callbacks.forEach(callback => callback(now)));
}

function visibility(value: boolean) {
  hidden = value;
  act(() => document.dispatchEvent(new Event('visibilitychange')));
}

beforeEach(() => {
  now = 0;
  hidden = false;
  reduced = false;
  nextFrame = 0;
  frames.clear();
  motion = new EventTarget();
  vi.spyOn(document, 'hidden', 'get').mockImplementation(() => hidden);
  vi.spyOn(window.performance, 'now').mockImplementation(() => now);
  vi.spyOn(window, 'requestAnimationFrame').mockImplementation(callback => {
    const id = ++nextFrame;
    frames.set(id, callback);
    return id;
  });
  vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(id => {
    frames.delete(id);
  });
  vi.spyOn(window, 'matchMedia').mockImplementation(query => ({
    get matches() {
      return reduced;
    },
    media: query,
    onchange: null,
    addEventListener: motion.addEventListener.bind(motion),
    removeEventListener: motion.removeEventListener.bind(motion),
    dispatchEvent: motion.dispatchEvent.bind(motion),
    addListener: vi.fn(),
    removeListener: vi.fn(),
  }));
});

afterEach(() => vi.restoreAllMocks());

describe('useMascotClock', () => {
  it('cancels hidden work and resumes the last frame without a time jump', () => {
    const { result, unmount } = renderHook(() => useMascotClock());
    frame(1000);
    expect(result.current).toBe(1);
    expect(frames.size).toBe(1);

    visibility(true);
    expect(frames.size).toBe(0);
    frame(60_000);
    expect(result.current).toBe(1);

    visibility(false);
    frame(1000);
    expect(result.current).toBe(2);
    expect(frames.size).toBe(1);
    unmount();
    expect(frames.size).toBe(0);
  });

  it('keeps a static frame for reduced motion, including live preference changes', () => {
    reduced = true;
    const { result } = renderHook(() => useMascotClock());
    expect(frames.size).toBe(0);
    expect(result.current).toBe(0);
    act(() => {
      reduced = false;
      motion.dispatchEvent(new Event('change'));
    });
    frame(500);
    expect(result.current).toBe(0.5);
    act(() => {
      reduced = true;
      motion.dispatchEvent(new Event('change'));
    });
    expect(frames.size).toBe(0);
  });

  it('does not wake an explicitly inactive clock when the page becomes visible', () => {
    hidden = true;
    const { rerender } = renderHook(({ active }) => useMascotClock(active), {
      initialProps: { active: false },
    });
    visibility(false);
    expect(frames.size).toBe(0);
    rerender({ active: true });
    expect(frames.size).toBe(1);
    rerender({ active: false });
    expect(frames.size).toBe(0);
  });
});
