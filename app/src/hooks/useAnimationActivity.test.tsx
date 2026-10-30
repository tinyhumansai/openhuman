import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useAnimationActivity } from './useAnimationActivity';

describe('useAnimationActivity', () => {
  let reduced = false;
  let hidden: ReturnType<typeof vi.spyOn>;
  let listeners = new Set<() => void>();
  let matchMedia: ReturnType<typeof vi.fn>;
  let addListener: ReturnType<typeof vi.fn>;
  let removeListener: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    reduced = false;
    listeners = new Set();
    addListener = vi.fn((_type: string, cb: () => void) => {
      listeners.add(cb);
    });
    removeListener = vi.fn((_type: string, cb: () => void) => {
      listeners.delete(cb);
    });
    matchMedia = vi.fn(() => ({
      get matches() {
        return reduced;
      },
      media: '(prefers-reduced-motion: reduce)',
      addEventListener: addListener,
      removeEventListener: removeListener,
    }));
    vi.stubGlobal('matchMedia', matchMedia);
    hidden = vi.spyOn(document, 'hidden', 'get').mockReturnValue(false);
  });

  afterEach(() => {
    hidden.mockRestore();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('pauses and resumes on tab visibility changes', () => {
    const { result } = renderHook(() => useAnimationActivity());
    expect(result.current).toBe(true);
    act(() => {
      hidden.mockReturnValue(true);
      document.dispatchEvent(new Event('visibilitychange'));
    });
    expect(result.current).toBe(false);
    act(() => {
      hidden.mockReturnValue(false);
      document.dispatchEvent(new Event('visibilitychange'));
    });
    expect(result.current).toBe(true);
  });

  it('respects reduced-motion changes without polling', () => {
    const { result, unmount } = renderHook(() => useAnimationActivity());
    expect(result.current).toBe(true);
    act(() => {
      reduced = true;
      listeners.forEach(cb => cb());
    });
    expect(result.current).toBe(false);
    act(() => {
      reduced = false;
      listeners.forEach(cb => cb());
    });
    expect(result.current).toBe(true);
    unmount();
    expect(removeListener).toHaveBeenCalled();
    expect(listeners.size).toBe(0);
  });
});
