/**
 * Brain graph — AUTOMATIC recovery from a transient failure (#5904).
 *
 * `Brain.errorRecovery.test.tsx` covers recovery that a user or an in-product
 * event triggers: manual Refresh, and the `openhuman:memory-tree-completed`
 * listener. Neither is automatic — before #5904 a failed load simply stayed
 * failed until something else happened, so a blip during a background refresh
 * left the panel stuck until the user noticed and pressed a button.
 *
 * What is pinned here is the ladder in `Brain.tsx` (`RETRY_DELAYS_MS`):
 * retries fire on their own, they are BOUNDED, a success resets them, and a
 * pending one is cancelled on unmount.
 *
 * Timers are faked because the delays are seconds long; `advanceTimersByTimeAsync`
 * is used rather than `advanceTimersByTime` so the promise inside each retry
 * settles before the assertion reads the call count.
 */
import { act, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import Brain from '../Brain';

const graphExportMock = vi.hoisted(() => vi.fn());
// Controllable authenticated identity so we can simulate a logout→login cycle
// (userId null → set) and assert the graph reloads (#4149).
const coreAuthRef = vi.hoisted(() => ({ current: 'user-A' as string | null }));
const navigateSpy = vi.hoisted(() => vi.fn());

vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => navigateSpy };
});

vi.mock('../../utils/tauriCommands', () => ({
  memoryTreeGraphExport: graphExportMock,
  isTauri: () => false,
}));

vi.mock('../../providers/CoreStateProvider', () => ({
  useCoreState: () => ({
    snapshot: {
      auth: { userId: coreAuthRef.current, isAuthenticated: coreAuthRef.current != null },
    },
  }),
}));

vi.mock('../../components/intelligence/MemoryGraph', async () => {
  const React = await import('react');
  return {
    MemoryGraph: ({ nodes }: { nodes: unknown[] }) =>
      React.createElement('div', { 'data-testid': 'memory-graph' }, `nodes:${nodes.length}`),
  };
});

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

vi.mock('../../components/layout/ChipTabs', async () => {
  const React = await import('react');
  return {
    default: ({ children }: { children?: React.ReactNode }) =>
      React.createElement('div', null, children),
  };
});
vi.mock('../../components/ui/BetaBanner', () => ({ default: () => null }));
vi.mock('../../components/intelligence/MemoryControls', () => ({ MemoryControls: () => null }));
vi.mock('../../components/intelligence/MemoryTreeStatusPanel', async () => {
  const React = await import('react');
  return {
    MemoryTreeStatusPanel: () => React.createElement('div', { 'data-testid': 'brain-sync' }),
  };
});
vi.mock('../../components/intelligence/MemorySourcesRegistry', async () => {
  const React = await import('react');
  return {
    MemorySourcesRegistry: () => React.createElement('div', { 'data-testid': 'brain-sources' }),
  };
});
vi.mock('../../components/intelligence/Toast', () => ({ ToastContainer: () => null }));
vi.mock('../../components/intelligence/SyncAuditPanel', async () => {
  const React = await import('react');
  return {
    SyncAuditPanel: () => React.createElement('div', { 'data-testid': 'brain-sync-audit' }),
  };
});

const makeGraph = (n: number) => ({
  nodes: Array.from({ length: n }, (_, i) => ({ id: `n${i}`, kind: 'summary', label: `N${i}` })),
  edges: [],
  content_root_abs: '/tmp/content',
});

/** Mirrors `RETRY_DELAYS_MS` in `Brain.tsx`. */
const RETRY_DELAYS_MS = [2_000, 4_000, 8_000];
const TOTAL_LADDER_MS = RETRY_DELAYS_MS.reduce((a, b) => a + b, 0);

const renderBrain = async () => {
  await act(async () => {
    renderWithProviders(<Brain />, { initialEntries: ['/?tab=graph'] });
  });
};

describe('Brain graph — automatic retry', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // `clearAllMocks` clears CALLS but not implementations, and several tests
    // here set a persistent `mockRejectedValue`. Without an explicit reset that
    // rejection leaks into the next test and silently changes its call counts.
    graphExportMock.mockReset();
    coreAuthRef.current = 'user-A';
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('retries a failed load on its own, with no user action', async () => {
    graphExportMock
      .mockRejectedValueOnce(new Error('transient blip'))
      .mockResolvedValue(makeGraph(2));

    await renderBrain();
    // One attempt so far, and it failed.
    expect(graphExportMock).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('memory-graph')).not.toBeInTheDocument();

    // Nothing is dispatched and nothing is clicked — only time passes.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(RETRY_DELAYS_MS[0]);
    });

    expect(graphExportMock).toHaveBeenCalledTimes(2);
    expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:2');
  });

  it('does not fire the retry before its delay has elapsed', async () => {
    // Otherwise "it retried" could be satisfied by an immediate re-fetch loop,
    // which is the thing a backoff exists to prevent.
    graphExportMock.mockRejectedValue(new Error('down'));

    await renderBrain();
    expect(graphExportMock).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(RETRY_DELAYS_MS[0] - 1);
    });
    expect(graphExportMock).toHaveBeenCalledTimes(1);
  });

  it('stops after the ladder is exhausted rather than retrying forever', async () => {
    graphExportMock.mockRejectedValue(new Error('backend is down'));

    await renderBrain();

    // Walk the whole ladder: 1 initial attempt + one per delay.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(TOTAL_LADDER_MS);
    });
    expect(graphExportMock).toHaveBeenCalledTimes(1 + RETRY_DELAYS_MS.length);

    // Far beyond it, the count must not move. This is the assertion that
    // distinguishes a bounded ladder from an infinite one.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(TOTAL_LADDER_MS * 10);
    });
    expect(graphExportMock).toHaveBeenCalledTimes(1 + RETRY_DELAYS_MS.length);
  });

  it('resets the ladder after a success, so a later failure retries again', async () => {
    // Fail, recover on the first retry, then fail again. If `attempt` were not
    // reset on success the second failure would resume mid-ladder and this
    // would come out one call short.
    graphExportMock
      .mockRejectedValueOnce(new Error('blip one'))
      .mockResolvedValueOnce(makeGraph(2))
      .mockRejectedValue(new Error('blip two'));

    await renderBrain();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(RETRY_DELAYS_MS[0]);
    });
    expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:2');
    expect(graphExportMock).toHaveBeenCalledTimes(2);

    // A second failure, triggered the in-product way.
    await act(async () => {
      window.dispatchEvent(new Event('openhuman:memory-tree-completed'));
    });
    expect(graphExportMock).toHaveBeenCalledTimes(3);
    // Prove the CATCH ran before advancing timers, not merely that the call was
    // made. The retry timer is armed inside the catch, so without this the
    // advance below could race the rejection's microtask and the ladder
    // assertion would be measuring an arming that had not happened yet.
    //
    // Asserted directly rather than through `waitFor`: this file runs on fake
    // timers, and `waitFor` polls on real timers that never advance, so it hangs
    // to the 30s test timeout instead of retrying. The enclosing `act` above has
    // already flushed the rejection's microtask, which is what makes the direct
    // read sound here.
    expect(document.querySelector('[data-slot="alert"]')).not.toBeNull();

    // The full ladder is available again: 3 more retries on top of that call.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(TOTAL_LADDER_MS);
    });
    expect(graphExportMock).toHaveBeenCalledTimes(3 + RETRY_DELAYS_MS.length);
  });

  it('keeps the failure visible while an automatic retry is in flight', async () => {
    // A timer-driven retry must not blank the alert for the duration of its
    // request. Nothing has changed from the user's point of view, so the
    // failure flickering out and back is noise they cannot account for. The
    // error is cleared on an accepted SUCCESS, which is when it stops being
    // true, rather than at the top of every retry.
    let resolveRetry!: (value: unknown) => void;
    const retryCall = new Promise(resolve => {
      resolveRetry = resolve;
    });
    graphExportMock
      .mockImplementationOnce(() => Promise.reject(new Error('down')))
      .mockImplementationOnce(() => retryCall);

    await renderBrain();
    // Direct, not `waitFor` — see the note in the ladder test: `waitFor` polls
    // on real timers and this file fakes them.
    expect(document.querySelector('[data-slot="alert"]')).not.toBeNull();

    // The automatic retry starts and stays pending.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(RETRY_DELAYS_MS[0]);
    });
    expect(graphExportMock).toHaveBeenCalledTimes(2);

    // Still on screen while that request is outstanding.
    expect(document.querySelector('[data-slot="alert"]')).not.toBeNull();

    // And it goes away only once the retry actually succeeds.
    await act(async () => {
      resolveRetry(makeGraph(2));
    });
    expect(document.querySelector('[data-slot="alert"]')).toBeNull();
    expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:2');
  });

  it('cancels a pending retry when the panel unmounts', async () => {
    // A timer that outlives the component would fetch (and setState) against an
    // unmounted tree.
    graphExportMock.mockRejectedValue(new Error('down'));

    let unmount!: () => void;
    await act(async () => {
      ({ unmount } = renderWithProviders(<Brain />, { initialEntries: ['/?tab=graph'] }));
    });
    expect(graphExportMock).toHaveBeenCalledTimes(1);

    unmount();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(TOTAL_LADDER_MS * 2);
    });
    expect(graphExportMock).toHaveBeenCalledTimes(1);
  });

  it('ignores a superseded FAILURE instead of retrying from it', async () => {
    // Two loads overlap: the initial one is still in flight when a
    // `memory-tree-completed` event starts a newer one that SUCCEEDS. When the
    // older call then rejects, that obsolete failure must not set an error or
    // schedule a retry against a graph that has already refreshed.
    let rejectFirst!: (reason: unknown) => void;
    const firstCall = new Promise((_resolve, reject) => {
      rejectFirst = reject;
    });
    graphExportMock
      .mockImplementationOnce(() => firstCall)
      .mockImplementationOnce(() => Promise.resolve(makeGraph(7)));

    await renderBrain();
    await act(async () => {
      window.dispatchEvent(new Event('openhuman:memory-tree-completed'));
    });
    expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:7');
    expect(graphExportMock).toHaveBeenCalledTimes(2);

    // The superseded call finally fails.
    await act(async () => {
      rejectFirst(new Error('older load failed'));
    });

    // No retry may come from it, however long we wait.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(TOTAL_LADDER_MS * 2);
    });
    expect(graphExportMock).toHaveBeenCalledTimes(2);
    expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:7');
  });

  it('renders an older success when the newer load FAILED, and still retries', async () => {
    // The collision between this PR's guard and #5942's "clear the error on an
    // accepted success". The guard's job is to stop a stale response clobbering
    // NEWER DATA — but when the newer request failed there is no newer data,
    // and dropping the older success leaves the user with an error and no
    // graph, which is worse than anything either PR intends.
    //
    // Both halves are asserted, because they are in tension and a fix that got
    // only one would look right:
    //   - the older success renders (the guard let it through), AND
    //   - the ladder is still armed, because the NEWEST thing we know about the
    //     backend is that it failed. Showing data and continuing to retry is
    //     the correct combination, not a leftover.
    let resolveFirst!: (value: unknown) => void;
    const firstCall = new Promise(resolve => {
      resolveFirst = resolve;
    });
    graphExportMock
      .mockImplementationOnce(() => firstCall)
      .mockImplementationOnce(() => Promise.reject(new Error('newer load failed')))
      .mockResolvedValue(makeGraph(6));

    await renderBrain();
    await act(async () => {
      window.dispatchEvent(new Event('openhuman:memory-tree-completed'));
    });
    expect(graphExportMock).toHaveBeenCalledTimes(2);

    // The superseded call succeeds. Nothing newer has rendered, so it must show.
    await act(async () => {
      resolveFirst(makeGraph(3));
    });
    expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:3');

    // And the retry the failure armed still fires.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(RETRY_DELAYS_MS[0]);
    });
    expect(graphExportMock).toHaveBeenCalledTimes(3);
    expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:6');
  });

  it('ignores a superseded SUCCESS instead of overwriting a newer graph', async () => {
    // The other direction of the same race: a slow older call resolving after a
    // newer one must not roll the graph back to its stale payload.
    let resolveFirst!: (value: unknown) => void;
    const firstCall = new Promise(resolve => {
      resolveFirst = resolve;
    });
    graphExportMock
      .mockImplementationOnce(() => firstCall)
      .mockImplementationOnce(() => Promise.resolve(makeGraph(9)));

    await renderBrain();
    await act(async () => {
      window.dispatchEvent(new Event('openhuman:memory-tree-completed'));
    });
    expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:9');

    await act(async () => {
      resolveFirst(makeGraph(1));
    });

    expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:9');
  });
});
