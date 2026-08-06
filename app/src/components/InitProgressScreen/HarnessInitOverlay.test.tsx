import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { CoreRpcError } from '../../services/coreRpcClient';
import type { HarnessInitSnapshot } from '../../services/harnessInitService';
import { renderWithProviders } from '../../test/test-utils';
// Imported after the mock is registered.
import HarnessInitOverlay from './HarnessInitOverlay';

// The overlay polls the service; drive it with a controllable mock.
const fetchHarnessInitStatus = vi.fn<() => Promise<HarnessInitSnapshot | null>>();
const runHarnessInit = vi.fn<(force?: boolean) => Promise<HarnessInitSnapshot | null>>();

vi.mock('../../services/harnessInitService', () => ({
  fetchHarnessInitStatus: () => fetchHarnessInitStatus(),
  runHarnessInit: (force?: boolean) => runHarnessInit(force),
}));

function snapshot(overrides: Partial<HarnessInitSnapshot> = {}): HarnessInitSnapshot {
  return {
    overall: 'running',
    startedAt: '2026-07-20T00:00:00Z',
    finishedAt: null,
    steps: [
      {
        id: 'python_runtime',
        label: 'Python runtime',
        required: false,
        state: 'running',
        message: null,
        percent: null,
        updatedAt: null,
      },
    ],
    ...overrides,
  };
}

beforeEach(() => {
  fetchHarnessInitStatus.mockReset();
  runHarnessInit.mockReset();
  window.sessionStorage.clear();
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe('HarnessInitOverlay', () => {
  it('renders nothing on a warm start (already done)', async () => {
    fetchHarnessInitStatus.mockResolvedValue(snapshot({ overall: 'done', startedAt: 'warm-run' }));

    const { container } = renderWithProviders(<HarnessInitOverlay />);

    await waitFor(() => expect(fetchHarnessInitStatus).toHaveBeenCalled());
    expect(screen.queryByText('Run in background')).not.toBeInTheDocument();
    expect(container).toBeEmptyDOMElement();
  });

  it('shows the blocking overlay while a provisioning run is in progress', async () => {
    fetchHarnessInitStatus.mockResolvedValue(snapshot({ startedAt: 'cold-run' }));

    renderWithProviders(<HarnessInitOverlay />);

    expect(await screen.findByText('Run in background')).toBeInTheDocument();
  });

  it('keeps the overlay dismissed across a remount for the same run (GH-5047)', async () => {
    const run = snapshot({ startedAt: 'same-run' });
    fetchHarnessInitStatus.mockResolvedValue(run);

    const first = renderWithProviders(<HarnessInitOverlay />);
    fireEvent.click(await screen.findByText('Run in background'));
    await waitFor(() => expect(screen.queryByText('Run in background')).not.toBeInTheDocument());
    first.unmount();

    // Remount while the same run is still in progress — it must not reopen.
    const second = renderWithProviders(<HarnessInitOverlay />);
    await waitFor(() => expect(fetchHarnessInitStatus).toHaveBeenCalled());
    // Give any pending poll a chance to (wrongly) re-render the overlay.
    await Promise.resolve();
    expect(screen.queryByText('Run in background')).not.toBeInTheDocument();
    expect(second.container).toBeEmptyDOMElement();
  });

  it('coalesces overlapping status polls into one RPC (StrictMode double-mount)', async () => {
    // Hold the fetch pending so both mounts' immediate polls overlap.
    let resolveFetch: (snap: HarnessInitSnapshot) => void = () => {};
    fetchHarnessInitStatus.mockImplementation(
      () =>
        new Promise<HarnessInitSnapshot>(resolve => {
          resolveFetch = resolve;
        })
    );

    // Two concurrent overlays stand in for the effect→cleanup→effect double-mount.
    renderWithProviders(<HarnessInitOverlay />);
    renderWithProviders(<HarnessInitOverlay />);

    // Both immediate polls should share a single in-flight request.
    expect(fetchHarnessInitStatus).toHaveBeenCalledTimes(1);

    resolveFetch(snapshot({ overall: 'done', startedAt: 'warm-run' }));
    await waitFor(() => expect(screen.queryByText('Run in background')).not.toBeInTheDocument());
  });

  it('reopens for a genuinely new provisioning run after a prior dismissal', async () => {
    // Dismiss the first run.
    fetchHarnessInitStatus.mockResolvedValue(snapshot({ startedAt: 'run-1' }));
    const first = renderWithProviders(<HarnessInitOverlay />);
    fireEvent.click(await screen.findByText('Run in background'));
    await waitFor(() => expect(screen.queryByText('Run in background')).not.toBeInTheDocument());
    first.unmount();

    // A new run (fresh startedAt) is allowed to surface again.
    fetchHarnessInitStatus.mockResolvedValue(snapshot({ startedAt: 'run-2' }));
    renderWithProviders(<HarnessInitOverlay />);
    expect(await screen.findByText('Run in background')).toBeInTheDocument();
  });

  // --- #5157: the poll loop must be bounded -------------------------------
  //
  // Before the fix, *any* status failure rescheduled the poll unconditionally
  // every 2s for the life of the window. Against a core that never serves the
  // method that was a permanent 30-calls-per-minute loop, and since the core
  // records each miss it produced ~9k Sentry events/day from one client.

  it('stops polling when the core does not expose harness_init_status (#5157)', async () => {
    vi.useFakeTimers();
    fetchHarnessInitStatus.mockRejectedValue(
      new CoreRpcError('unknown method: openhuman.harness_init_status', 'method_not_found')
    );

    const { container } = renderWithProviders(<HarnessInitOverlay />);

    // Let the immediate poll settle, then run well past many poll intervals.
    await vi.advanceTimersByTimeAsync(0);
    expect(fetchHarnessInitStatus).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(120_000);

    // Permanently absent ⇒ exactly one attempt, ever. No retry, no overlay.
    expect(fetchHarnessInitStatus).toHaveBeenCalledTimes(1);
    expect(container).toBeEmptyDOMElement();
  });

  it('gives up after a bounded number of consecutive transient failures (#5157)', async () => {
    vi.useFakeTimers();
    // A persistent non-method-not-found fault (core wedged, transport down).
    fetchHarnessInitStatus.mockRejectedValue(new Error('error sending request for url'));

    renderWithProviders(<HarnessInitOverlay />);

    // 5 attempts, backing off 2s → 4s → 8s → 16s between them (30s total).
    await vi.advanceTimersByTimeAsync(120_000);
    expect(fetchHarnessInitStatus).toHaveBeenCalledTimes(5);

    // And it stays stopped rather than resuming later.
    await vi.advanceTimersByTimeAsync(600_000);
    expect(fetchHarnessInitStatus).toHaveBeenCalledTimes(5);
  });

  // Review follow-up on #5157: the failure cap must not strand the *blocking*
  // overlay. If the core has a transient outage that outlasts the budget while
  // a `running` snapshot is on screen, giving up would pin the app behind stale
  // progress for the rest of the session — the pre-#5157 loop recovered from
  // exactly that. The cap still applies when nothing blocking is displayed
  // (covered by the give-up test above); here the loop drops to a slow cadence
  // instead of stopping.
  it('keeps watching a running overlay through an outage longer than the failure budget', async () => {
    vi.useFakeTimers();
    fetchHarnessInitStatus
      // A provisioning run is live — the overlay is now blocking the app.
      .mockResolvedValueOnce(snapshot({ startedAt: 'stall-run' }))
      // The core drops out for well past MAX_TRANSIENT_FAILURES attempts.
      .mockRejectedValueOnce(new Error('error sending request for url'))
      .mockRejectedValueOnce(new Error('error sending request for url'))
      .mockRejectedValueOnce(new Error('error sending request for url'))
      .mockRejectedValueOnce(new Error('error sending request for url'))
      .mockRejectedValueOnce(new Error('error sending request for url'))
      .mockRejectedValueOnce(new Error('error sending request for url'))
      .mockRejectedValueOnce(new Error('error sending request for url'))
      .mockRejectedValueOnce(new Error('error sending request for url'))
      // ...and then comes back, having finished the run.
      .mockResolvedValue(
        snapshot({ overall: 'done', startedAt: 'stall-run', finishedAt: '2026-07-20T00:05:00Z' })
      );

    renderWithProviders(<HarnessInitOverlay />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(screen.getByText('Run in background')).toBeInTheDocument();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300_000);
    });

    // It kept polling past the 5-attempt cap instead of freezing...
    expect(fetchHarnessInitStatus.mock.calls.length).toBeGreaterThan(6);
    // ...so the recovered core's terminal snapshot was observed and the
    // blocking overlay cleared itself. (Asserted directly rather than through
    // `waitFor`, which would wait on real timers while fake ones are installed.)
    expect(screen.queryByText('Run in background')).not.toBeInTheDocument();
  });

  it('keeps polling after a transient failure while the core is still booting', async () => {
    vi.useFakeTimers();
    // The legitimate cold-start case the retry exists for: fail once, then the
    // core comes up. The failure budget must reset on success, not leak.
    fetchHarnessInitStatus
      .mockRejectedValueOnce(new Error('error sending request for url'))
      .mockResolvedValue(snapshot({ startedAt: 'cold-run' }));

    renderWithProviders(<HarnessInitOverlay />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });

    // Retried once and recovered — the overlay renders the live run.
    expect(fetchHarnessInitStatus).toHaveBeenCalledTimes(2);
    expect(screen.getByText('Run in background')).toBeInTheDocument();
  });
});
