import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useDebouncedValue } from '../useDebouncedValue';

describe('useDebouncedValue', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('returns the initial value immediately', () => {
    const value = { query: 'initial' };
    const { result } = renderHook(() => useDebouncedValue(value, 100));

    expect(result.current).toBe(value);
  });

  it('updates only after the delay elapses', () => {
    const { result, rerender } = renderHook(
      ({ value, delayMs }) => useDebouncedValue(value, delayMs),
      { initialProps: { value: 'first', delayMs: 100 } }
    );

    rerender({ value: 'second', delayMs: 100 });
    act(() => vi.advanceTimersByTime(99));
    expect(result.current).toBe('first');

    act(() => vi.advanceTimersByTime(1));
    expect(result.current).toBe('second');
  });

  it('replaces a pending update with the latest value', () => {
    const { result, rerender } = renderHook(({ value }) => useDebouncedValue(value, 100), {
      initialProps: { value: 'first' },
    });

    rerender({ value: 'second' });
    act(() => vi.advanceTimersByTime(75));
    rerender({ value: 'third' });
    act(() => vi.advanceTimersByTime(25));
    expect(result.current).toBe('first');

    act(() => vi.advanceTimersByTime(75));
    expect(result.current).toBe('third');
  });

  it('restarts a pending update when the delay changes', () => {
    const { result, rerender } = renderHook(
      ({ value, delayMs }) => useDebouncedValue(value, delayMs),
      { initialProps: { value: 'first', delayMs: 100 } }
    );

    rerender({ value: 'second', delayMs: 100 });
    act(() => vi.advanceTimersByTime(75));
    rerender({ value: 'second', delayMs: 50 });
    act(() => vi.advanceTimersByTime(49));
    expect(result.current).toBe('first');

    act(() => vi.advanceTimersByTime(1));
    expect(result.current).toBe('second');
  });

  it('preserves referential values', () => {
    const initial = { query: 'first' };
    const next = { query: 'second' };
    const { result, rerender } = renderHook(({ value }) => useDebouncedValue(value, 100), {
      initialProps: { value: initial },
    });

    rerender({ value: next });
    expect(result.current).toBe(initial);

    act(() => vi.advanceTimersByTime(100));
    expect(result.current).toBe(next);
  });

  it('preserves callable values without invoking them', () => {
    const initial = vi.fn(() => 'initial result');
    const next = vi.fn(() => 'next result');
    const { result, rerender } = renderHook(({ value }) => useDebouncedValue(value, 100), {
      initialProps: { value: initial },
    });

    expect(result.current).toBe(initial);
    expect(initial).not.toHaveBeenCalled();

    rerender({ value: next });
    act(() => vi.advanceTimersByTime(100));

    expect(result.current).toBe(next);
    expect(initial).not.toHaveBeenCalled();
    expect(next).not.toHaveBeenCalled();
  });

  it('cleans up a pending update on unmount', () => {
    const clearTimeoutSpy = vi.spyOn(window, 'clearTimeout');
    const { rerender, unmount } = renderHook(({ value }) => useDebouncedValue(value, 100), {
      initialProps: { value: 'first' },
    });

    rerender({ value: 'second' });
    clearTimeoutSpy.mockClear();
    unmount();

    expect(clearTimeoutSpy).toHaveBeenCalledTimes(1);
  });

  it('normalizes a negative delay to zero', () => {
    const { result, rerender } = renderHook(({ value }) => useDebouncedValue(value, -50), {
      initialProps: { value: 'first' },
    });

    rerender({ value: 'second' });
    expect(result.current).toBe('first');

    act(() => vi.advanceTimersByTime(0));
    expect(result.current).toBe('second');
  });
});
