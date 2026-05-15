/**
 * Tests for coreHealthMonitor — covers changed lines 17-19, 21-23, 25-29,
 * 31-34, 37, 41-42, 47-50, 53-57, 60-64.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Mock store and connectivitySlice first.
const dispatchMock = vi.fn();
vi.mock('../../store/index', () => ({
  store: { dispatch: dispatchMock, getState: () => ({ connectivity: { core: 'reachable' } }) },
}));

const setCoreMock = vi.fn((payload: unknown) => ({ type: 'connectivity/setCore', payload }));
vi.mock('../../store/connectivitySlice', () => ({ setCore: (p: unknown) => setCoreMock(p) }));

const callCoreRpcMock = vi.fn();
vi.mock('../coreRpcClient', () => ({ callCoreRpc: callCoreRpcMock }));

/** Flush all pending microtasks (resolved promises). */
async function flushPromises(): Promise<void> {
  // Multiple rounds handle chained .then() callbacks.
  for (let i = 0; i < 10; i++) {
    await Promise.resolve();
  }
}

describe('coreHealthMonitor', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.resetModules();
    dispatchMock.mockClear();
    setCoreMock.mockClear();
    callCoreRpcMock.mockClear();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('startCoreHealthMonitor probes immediately on start (lines 53-57)', async () => {
    callCoreRpcMock.mockResolvedValueOnce({});

    const { startCoreHealthMonitor, stopCoreHealthMonitor } = await import('../coreHealthMonitor');
    startCoreHealthMonitor();

    // Flush micro-tasks so the async probe runs.
    await flushPromises();

    expect(callCoreRpcMock).toHaveBeenCalledWith(
      expect.objectContaining({ method: 'openhuman.connectivity_diag' })
    );
    stopCoreHealthMonitor();
  });

  it('dispatches reachable on successful probe (lines 25-29)', async () => {
    callCoreRpcMock.mockResolvedValueOnce({});

    const { startCoreHealthMonitor, stopCoreHealthMonitor } = await import('../coreHealthMonitor');
    startCoreHealthMonitor();
    await flushPromises();

    expect(setCoreMock).toHaveBeenCalledWith({ value: 'reachable' });
    stopCoreHealthMonitor();
  });

  it('does not dispatch unreachable until FAIL_THRESHOLD consecutive failures (lines 31-34)', async () => {
    // First failure — below threshold (2), should NOT dispatch unreachable yet.
    callCoreRpcMock.mockRejectedValueOnce(new Error('ECONNREFUSED'));

    const { startCoreHealthMonitor, stopCoreHealthMonitor } = await import('../coreHealthMonitor');
    startCoreHealthMonitor();
    await flushPromises();

    // Only 1 failure, threshold is 2 — unreachable must NOT have been dispatched.
    const unreachableCalls = setCoreMock.mock.calls.filter(
      ([arg]) => (arg as { value: string }).value === 'unreachable'
    );
    expect(unreachableCalls).toHaveLength(0);
    stopCoreHealthMonitor();
  });

  it('dispatches unreachable after FAIL_THRESHOLD consecutive failures (lines 31-34)', async () => {
    // Two consecutive failures → should cross the threshold.
    callCoreRpcMock
      .mockRejectedValueOnce(new Error('ECONNREFUSED first'))
      .mockRejectedValueOnce(new Error('ECONNREFUSED second'));

    const { startCoreHealthMonitor, stopCoreHealthMonitor } = await import('../coreHealthMonitor');
    startCoreHealthMonitor();

    // First probe.
    await flushPromises();

    // Advance timer to trigger the degraded-mode 5 s retry.
    vi.advanceTimersByTime(5_001);
    await flushPromises();

    const unreachableCalls = setCoreMock.mock.calls.filter(
      ([arg]) => (arg as { value: string }).value === 'unreachable'
    );
    expect(unreachableCalls.length).toBeGreaterThanOrEqual(1);
    stopCoreHealthMonitor();
  });

  it('is idempotent — second startCoreHealthMonitor call is a no-op (lines 53-54)', async () => {
    callCoreRpcMock.mockResolvedValue({});

    const { startCoreHealthMonitor, stopCoreHealthMonitor } = await import('../coreHealthMonitor');
    startCoreHealthMonitor();
    startCoreHealthMonitor(); // second call must not double-probe

    await flushPromises();

    // Only 1 probe should have fired.
    expect(callCoreRpcMock).toHaveBeenCalledTimes(1);
    stopCoreHealthMonitor();
  });

  it('stopCoreHealthMonitor prevents further scheduling (lines 60-64)', async () => {
    callCoreRpcMock.mockResolvedValue({});

    const { startCoreHealthMonitor, stopCoreHealthMonitor } = await import('../coreHealthMonitor');
    startCoreHealthMonitor();
    await flushPromises();

    const firstCallCount = callCoreRpcMock.mock.calls.length;
    stopCoreHealthMonitor();

    // Advancing time should not trigger another probe.
    vi.advanceTimersByTime(60_000);
    await flushPromises();

    expect(callCoreRpcMock).toHaveBeenCalledTimes(firstCallCount);
  });

  it('schedule picks degraded interval when consecutiveFails > 0 (lines 41-42, 47-50)', async () => {
    // Make probe fail once so consecutiveFails becomes 1.
    callCoreRpcMock.mockRejectedValueOnce(new Error('connection refused')).mockResolvedValue({});

    const { startCoreHealthMonitor, stopCoreHealthMonitor } = await import('../coreHealthMonitor');
    startCoreHealthMonitor();
    await flushPromises();

    // After 1 failure the next poll should be at DEGRADED_INTERVAL_MS = 5s, not 30s.
    vi.advanceTimersByTime(5_001);
    await flushPromises();

    // Second probe should have fired (recovery check).
    expect(callCoreRpcMock).toHaveBeenCalledTimes(2);
    stopCoreHealthMonitor();
  });

  it('error message is extracted from Error instance (lines 31-34)', async () => {
    callCoreRpcMock
      .mockRejectedValueOnce(new Error('timeout msg'))
      .mockRejectedValueOnce(new Error('timeout msg'));

    const { startCoreHealthMonitor, stopCoreHealthMonitor } = await import('../coreHealthMonitor');
    startCoreHealthMonitor();
    await flushPromises();

    vi.advanceTimersByTime(5_001);
    await flushPromises();

    const unreachableCall = setCoreMock.mock.calls.find(
      ([arg]) => (arg as { value: string }).value === 'unreachable'
    );
    expect(unreachableCall).toBeDefined();
    expect((unreachableCall![0] as { error: string }).error).toBe('timeout msg');
    stopCoreHealthMonitor();
  });

  it('error message falls back to String(err) when not an Error instance (lines 31-34)', async () => {
    callCoreRpcMock
      .mockRejectedValueOnce('plain string error')
      .mockRejectedValueOnce('plain string error');

    const { startCoreHealthMonitor, stopCoreHealthMonitor } = await import('../coreHealthMonitor');
    startCoreHealthMonitor();
    await flushPromises();

    vi.advanceTimersByTime(5_001);
    await flushPromises();

    const unreachableCall = setCoreMock.mock.calls.find(
      ([arg]) => (arg as { value: string }).value === 'unreachable'
    );
    expect(unreachableCall).toBeDefined();
    expect((unreachableCall![0] as { error: string }).error).toBe('plain string error');
    stopCoreHealthMonitor();
  });
});
