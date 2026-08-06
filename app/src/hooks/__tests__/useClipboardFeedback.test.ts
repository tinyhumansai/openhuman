import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import { useClipboardFeedback } from '../useClipboardFeedback';

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe('useClipboardFeedback', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  test('uses navigator.clipboard by default and resets copied feedback after 2000ms', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    const { result } = renderHook(() => useClipboardFeedback());

    let copied = false;
    await act(async () => {
      copied = await result.current.copy('shareable value');
    });

    expect(copied).toBe(true);
    expect(writeText).toHaveBeenCalledWith('shareable value');
    expect(result.current.status).toBe('copied');

    act(() => {
      vi.advanceTimersByTime(1999);
    });
    expect(result.current.status).toBe('copied');

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(result.current.status).toBe('idle');
  });

  test('returns false and resets error feedback after the configured delay', async () => {
    const writeText = vi.fn().mockRejectedValue(new Error('clipboard unavailable'));
    const { result } = renderHook(() => useClipboardFeedback({ writeText, resetAfterMs: 600 }));

    let copied = true;
    await act(async () => {
      copied = await result.current.copy('shareable value');
    });

    expect(copied).toBe(false);
    expect(result.current.status).toBe('error');

    act(() => {
      vi.advanceTimersByTime(599);
    });
    expect(result.current.status).toBe('error');
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(result.current.status).toBe('idle');
  });

  test('returns false when navigator.clipboard is missing and resets error after 2000ms', async () => {
    Object.defineProperty(navigator, 'clipboard', { value: undefined, configurable: true });
    const { result } = renderHook(() => useClipboardFeedback());

    let copied = true;
    await act(async () => {
      copied = await result.current.copy('shareable value');
    });

    expect(copied).toBe(false);
    expect(result.current.status).toBe('error');
    act(() => {
      vi.advanceTimersByTime(1999);
    });
    expect(result.current.status).toBe('error');
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(result.current.status).toBe('idle');
  });

  test('replaces the reset timer after a repeated successful copy', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    const { result } = renderHook(() => useClipboardFeedback({ writeText, resetAfterMs: 1000 }));

    await act(async () => {
      await result.current.copy('first value');
    });
    act(() => {
      vi.advanceTimersByTime(750);
    });
    await act(async () => {
      await result.current.copy('second value');
    });
    act(() => {
      vi.advanceTimersByTime(750);
    });
    expect(result.current.status).toBe('copied');

    act(() => {
      vi.advanceTimersByTime(250);
    });
    expect(result.current.status).toBe('idle');
  });

  test('replaces an error reset timer when a newer copy fails', async () => {
    const writeText = vi.fn().mockRejectedValue(new Error('clipboard unavailable'));
    const { result } = renderHook(() => useClipboardFeedback({ writeText, resetAfterMs: 1000 }));

    await act(async () => {
      await result.current.copy('first value');
    });
    act(() => {
      vi.advanceTimersByTime(750);
    });
    await act(async () => {
      await result.current.copy('second value');
    });

    act(() => {
      vi.advanceTimersByTime(999);
    });
    expect(result.current.status).toBe('error');
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(result.current.status).toBe('idle');
  });

  test('manual reset clears feedback and prevents an in-flight copy from restoring it', async () => {
    const pending = deferred<void>();
    const writeText = vi.fn().mockReturnValue(pending.promise);
    const { result } = renderHook(() => useClipboardFeedback({ writeText }));

    let copyResult: Promise<boolean>;
    act(() => {
      copyResult = result.current.copy('shareable value');
    });
    act(() => {
      result.current.reset();
    });
    expect(result.current.status).toBe('idle');

    pending.resolve();
    await act(async () => {
      await copyResult;
    });
    expect(result.current.status).toBe('idle');
  });

  test('manual reset clears copied feedback and cancels its pending timer', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    const { result } = renderHook(() => useClipboardFeedback({ writeText, resetAfterMs: 1000 }));

    await act(async () => {
      await result.current.copy('shareable value');
    });
    expect(result.current.status).toBe('copied');

    act(() => {
      result.current.reset();
      vi.advanceTimersByTime(1000);
    });
    expect(result.current.status).toBe('idle');
  });

  test('manual reset clears error feedback and cancels its pending timer', async () => {
    const writeText = vi.fn().mockRejectedValue(new Error('clipboard unavailable'));
    const { result } = renderHook(() => useClipboardFeedback({ writeText, resetAfterMs: 1000 }));

    await act(async () => {
      await result.current.copy('shareable value');
    });
    expect(result.current.status).toBe('error');
    expect(vi.getTimerCount()).toBe(1);

    act(() => {
      result.current.reset();
    });
    expect(result.current.status).toBe('idle');
    expect(vi.getTimerCount()).toBe(0);
  });

  test('does not update state when an in-flight copy settles after unmount', async () => {
    const pending = deferred<void>();
    const writeText = vi.fn().mockReturnValue(pending.promise);
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    const { result, unmount } = renderHook(() => useClipboardFeedback({ writeText }));

    let copyResult: Promise<boolean>;
    act(() => {
      copyResult = result.current.copy('shareable value');
    });
    unmount();
    pending.resolve();
    await act(async () => {
      await copyResult;
    });

    expect(consoleError).not.toHaveBeenCalled();
    expect(result.current.status).toBe('idle');
  });

  test('clears a scheduled error reset on unmount', async () => {
    const writeText = vi.fn().mockRejectedValue(new Error('clipboard unavailable'));
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    const { result, unmount } = renderHook(() => useClipboardFeedback({ writeText }));

    await act(async () => {
      await result.current.copy('shareable value');
    });
    expect(result.current.status).toBe('error');
    expect(vi.getTimerCount()).toBe(1);

    unmount();
    expect(vi.getTimerCount()).toBe(0);
    act(() => {
      vi.advanceTimersByTime(2000);
    });
    expect(consoleError).not.toHaveBeenCalled();
  });

  test('keeps callbacks stable while honoring updated writer and reset duration options', async () => {
    const firstWriter = vi.fn().mockResolvedValue(undefined);
    const secondWriter = vi.fn().mockResolvedValue(undefined);
    const { result, rerender } = renderHook(
      ({ writeText, resetAfterMs }) => useClipboardFeedback({ writeText, resetAfterMs }),
      { initialProps: { writeText: firstWriter, resetAfterMs: 1000 } }
    );
    const initialCopy = result.current.copy;
    const initialReset = result.current.reset;

    rerender({ writeText: secondWriter, resetAfterMs: 250 });
    expect(result.current.copy).toBe(initialCopy);
    expect(result.current.reset).toBe(initialReset);

    await act(async () => {
      await result.current.copy('latest options');
    });
    expect(firstWriter).not.toHaveBeenCalled();
    expect(secondWriter).toHaveBeenCalledWith('latest options');

    act(() => {
      vi.advanceTimersByTime(249);
    });
    expect(result.current.status).toBe('copied');
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(result.current.status).toBe('idle');
  });

  test('only lets the latest overlapping copy update feedback', async () => {
    const first = deferred<void>();
    const second = deferred<void>();
    const writeText = vi
      .fn()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);
    const { result } = renderHook(() => useClipboardFeedback({ writeText }));

    let firstResult: Promise<boolean>;
    let secondResult: Promise<boolean>;
    act(() => {
      firstResult = result.current.copy('first value');
      secondResult = result.current.copy('second value');
    });

    second.reject(new Error('latest failed'));
    await act(async () => {
      expect(await secondResult).toBe(false);
    });
    expect(result.current.status).toBe('error');

    first.resolve();
    await act(async () => {
      expect(await firstResult).toBe(true);
    });
    expect(result.current.status).toBe('error');
  });
});
