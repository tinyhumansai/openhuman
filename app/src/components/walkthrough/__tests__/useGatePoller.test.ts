import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { InteractiveGate } from '../interactiveGates';
import { useGatePoller } from '../useGatePoller';

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

function makeGate(overrides: Partial<InteractiveGate> = {}): InteractiveGate {
  return {
    id: 'test-gate',
    label: 'Do the thing',
    skipLabel: 'Skip',
    isComplete: vi.fn(() => false),
    pollIntervalMs: 500,
    ...overrides,
  };
}

describe('useGatePoller', () => {
  it('returns true when gate is null (non-gated step)', () => {
    const { result } = renderHook(() => useGatePoller(null));
    expect(result.current).toBe(true);
  });

  it('returns true immediately when gate is already complete', () => {
    const gate = makeGate({ isComplete: () => true });
    const { result } = renderHook(() => useGatePoller(gate));
    expect(result.current).toBe(true);
  });

  it('returns false initially when gate is incomplete', () => {
    const gate = makeGate({ isComplete: () => false });
    const { result } = renderHook(() => useGatePoller(gate));
    // After the effect runs, it should be false
    act(() => {
      vi.advanceTimersByTime(0);
    });
    expect(result.current).toBe(false);
  });

  it('transitions to true when gate completes during polling', () => {
    let complete = false;
    const gate = makeGate({ isComplete: () => complete, pollIntervalMs: 500 });
    const { result } = renderHook(() => useGatePoller(gate));

    act(() => {
      vi.advanceTimersByTime(0);
    });
    expect(result.current).toBe(false);

    // Simulate the action being completed
    complete = true;

    act(() => {
      vi.advanceTimersByTime(500);
    });
    expect(result.current).toBe(true);
  });

  it('stops polling after gate completes', () => {
    let complete = false;
    const isCompleteFn = vi.fn(() => complete);
    const gate = makeGate({ isComplete: isCompleteFn, pollIntervalMs: 500 });

    renderHook(() => useGatePoller(gate));

    act(() => {
      vi.advanceTimersByTime(0);
    });

    // Complete the gate
    complete = true;
    act(() => {
      vi.advanceTimersByTime(500);
    });

    // Record call count after completion
    const callCountAfterComplete = isCompleteFn.mock.calls.length;

    // Advance more — should not call isComplete again
    act(() => {
      vi.advanceTimersByTime(2000);
    });
    expect(isCompleteFn.mock.calls.length).toBe(callCountAfterComplete);
  });

  it('uses default poll interval when not specified', () => {
    let complete = false;
    const isCompleteFn = vi.fn(() => complete);
    const gate = makeGate({ isComplete: isCompleteFn });
    delete (gate as any).pollIntervalMs;

    renderHook(() => useGatePoller(gate));

    act(() => {
      vi.advanceTimersByTime(0);
    });
    const callsAfterInit = isCompleteFn.mock.calls.length;

    // Default is 1000ms
    act(() => {
      vi.advanceTimersByTime(999);
    });
    expect(isCompleteFn.mock.calls.length).toBe(callsAfterInit);

    complete = true;
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(isCompleteFn.mock.calls.length).toBeGreaterThan(callsAfterInit);
  });

  it('cleans up interval on unmount', () => {
    const gate = makeGate({ isComplete: () => false, pollIntervalMs: 500 });
    const { unmount } = renderHook(() => useGatePoller(gate));

    act(() => {
      vi.advanceTimersByTime(0);
    });
    unmount();

    // No errors should be thrown when timers fire after unmount
    act(() => {
      vi.advanceTimersByTime(2000);
    });
  });
});
