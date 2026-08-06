import { act, renderHook } from '@testing-library/react';
import { describe, expect, test } from 'vitest';

import { useLatestAsync } from '../useLatestAsync';

describe('useLatestAsync', () => {
  test('returns monotonically increasing generations and accepts only the newest', () => {
    const { result } = renderHook(() => useLatestAsync());

    let first!: number;
    let second!: number;
    act(() => {
      first = result.current.begin();
      second = result.current.begin();
    });

    expect(second).toBeGreaterThan(first);
    expect(result.current.isLatest(first)).toBe(false);
    expect(result.current.isLatest(second)).toBe(true);
  });

  test('invalidate makes every prior generation stale', () => {
    const { result } = renderHook(() => useLatestAsync());

    let generation!: number;
    act(() => {
      generation = result.current.begin();
      result.current.invalidate();
    });

    expect(result.current.isLatest(generation)).toBe(false);
  });

  test('unmount invalidates outstanding work', () => {
    const { result, unmount } = renderHook(() => useLatestAsync());
    const guard = result.current;

    let generation!: number;
    act(() => {
      generation = guard.begin();
    });
    unmount();

    expect(guard.isLatest(generation)).toBe(false);
  });

  test('returns referentially stable methods across renders', () => {
    const { result, rerender } = renderHook(() => useLatestAsync());
    const initial = result.current;

    rerender();

    expect(result.current.begin).toBe(initial.begin);
    expect(result.current.isLatest).toBe(initial.isLatest);
    expect(result.current.invalidate).toBe(initial.invalidate);
  });
});
