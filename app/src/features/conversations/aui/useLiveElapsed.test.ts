import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useLiveElapsed, useRunningSince } from './useLiveElapsed';

describe('useLiveElapsed / useRunningSince', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 9, 7, 12, 0, 0));
  });
  afterEach(() => vi.useRealTimers());

  it('ticks while running and stops when it settles', () => {
    const { result, rerender } = renderHook(
      ({ running }) => useLiveElapsed(useRunningSince(running), running),
      { initialProps: { running: true } }
    );
    expect(result.current).toBe(0);
    act(() => {
      vi.advanceTimersByTime(3_000);
    });
    expect(result.current).toBe(3_000);
    rerender({ running: false });
    expect(result.current).toBeUndefined();
  });

  it('is undefined without a start time', () => {
    const { result } = renderHook(() => useLiveElapsed(undefined, true));
    expect(result.current).toBeUndefined();
  });
});
