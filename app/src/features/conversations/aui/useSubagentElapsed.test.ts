import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { resetSubagentElapsedAnchors, useSubagentElapsed } from './useSubagentElapsed';

describe('useSubagentElapsed', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 9, 7, 12, 0, 0));
    resetSubagentElapsedAnchors();
  });
  afterEach(() => vi.useRealTimers());

  it('ticks every second while running and keeps its anchor across remounts', () => {
    const first = renderHook(() => useSubagentElapsed('sub-1', true, undefined));
    expect(first.result.current).toBe(0);
    act(() => {
      vi.advanceTimersByTime(3000);
    });
    expect(first.result.current).toBe(3000);
    first.unmount();

    act(() => {
      vi.advanceTimersByTime(2000);
    });
    const second = renderHook(() => useSubagentElapsed('sub-1', true, undefined));
    expect(second.result.current).toBe(5000);
  });

  it('reports the settled duration once finished', () => {
    const { result } = renderHook(() => useSubagentElapsed('sub-2', false, 64_000));
    expect(result.current).toBe(64_000);
  });

  it('is unknown while running without a task id', () => {
    const { result } = renderHook(() => useSubagentElapsed(undefined, true, undefined));
    expect(result.current).toBeUndefined();
  });
});
