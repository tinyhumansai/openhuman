/**
 * Brain graph — transient-failure recovery, and what a failed refresh shows.
 *
 * `Brain.test.tsx` already covers "an alert appears when the fetch fails".
 * None of the behaviours here is covered anywhere:
 *
 *   1. A transient failure must CLEAR on the next successful load. This is the
 *      accurate half of the "Couldn't load your brain" report — the panel is
 *      recoverable (`Brain.tsx:97` calls `setError(null)` at the top of every
 *      `load()`, and `MemoryControls` renders above the error branch so Refresh
 *      stays reachable), but nothing pinned that, so deleting that one line
 *      would have turned a recoverable error into a permanent one silently.
 *
 *   2. A refresh that fails AFTER a successful load keeps the stale graph and
 *      warns. This was BUG-W4-3 (#5895): the alert branch was reachable only
 *      while `graph` was null, and the catch in `load()` never clears `graph`,
 *      so a failed refresh left stale data on screen with no indication. The
 *      second test used to CHARACTERISE that swallow; it now asserts the
 *      warning, which is the same test inverted by the fix.
 *
 *   3. A FIRST load that fails still shows the destructive "couldn't load"
 *      alert and no graph — the branch that must not be swallowed by (2).
 *
 *   4. A failure whose message is EMPTY still surfaces. `load()` stores
 *      `err.message`, so `new Error('')` becomes `''`; a truthiness test would
 *      swallow it silently, which is (2) again through a different door.
 *
 *   5. An older load succeeding after a newer one failed must NOT leave a
 *      stale-data warning on data that is not stale. The two overlap and share
 *      state; the warning is what turns that pre-existing race into a visible
 *      false alarm, so the success path clears the error.
 *
 * These drive the refetch through the `openhuman:memory-tree-completed` window
 * event, which is the in-product refetch trigger and needs no DOM control.
 */
import { act, screen, waitFor } from '@testing-library/react';
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

describe('Brain graph — transient failure recovery', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    coreAuthRef.current = 'user-A';
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('clears the error when a retry succeeds after a transient failure', async () => {
    // Fail once, then succeed — a transient blip, not a broken backend.
    graphExportMock
      .mockRejectedValueOnce(new Error('transient blip'))
      .mockResolvedValue(makeGraph(2));

    await act(async () => {
      renderWithProviders(<Brain />, { initialEntries: ['/?tab=graph'] });
    });

    // The failure is visible first — otherwise the clearing below proves nothing.
    await waitFor(() => {
      expect(screen.getByRole('alert')).toBeInTheDocument();
    });
    expect(screen.queryByTestId('memory-graph')).not.toBeInTheDocument();

    // The in-product refetch trigger.
    await act(async () => {
      window.dispatchEvent(new Event('openhuman:memory-tree-completed'));
    });

    // The panel RECOVERS: the retry runs and its data renders.
    //
    // This comment used to say the test could not prove `error` itself was
    // cleared — once `graph` was truthy, a still-set `error` was simply not in
    // the DOM, so deleting `setError(null)` from `Brain.tsx` left both tests in
    // this file passing. That was true, and it was a consequence of BUG-W4-3.
    //
    // Fixing #5895 makes the state reset OBSERVABLE, which is a second benefit
    // of the fix worth naming: a stale `error` alongside a loaded graph now
    // renders the warning variant. So the absence of that warning after a
    // successful reload is real evidence that `setError(null)` ran, and the
    // assertion below can fail if it is removed.
    //
    // Revert-checked by disabling the `openhuman:memory-tree-completed`
    // listener (`Brain.tsx:113-117`): both tests then fail on the call-count
    // assertion below.
    await waitFor(() => {
      expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:2');
    });
    expect(graphExportMock).toHaveBeenCalledTimes(2);
    // The error state was genuinely cleared, not merely hidden: a surviving
    // `error` beside a loaded graph would render the stale-data warning.
    expect(document.querySelector('[data-variant="warning"]')).toBeNull();
  });

  it('warns and keeps the stale graph when a refresh fails after a good load', async () => {
    // Succeed first, then fail — the opposite order to the test above.
    graphExportMock
      .mockResolvedValueOnce(makeGraph(3))
      .mockRejectedValue(new Error('refresh blew up'));

    await act(async () => {
      renderWithProviders(<Brain />, { initialEntries: ['/?tab=graph'] });
    });
    await waitFor(() => {
      expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:3');
    });

    await act(async () => {
      window.dispatchEvent(new Event('openhuman:memory-tree-completed'));
    });
    await waitFor(() => {
      expect(graphExportMock).toHaveBeenCalledTimes(2);
    });

    // This test previously asserted the OPPOSITE — `queryByRole('alert')` was
    // expected to be absent — as a deliberate characterisation of BUG-W4-3,
    // with a note saying it must be rewritten "to assert whichever was chosen"
    // once the bug was fixed. #5895 chose: warn and keep the stale graph. So
    // this is that rewrite, and the test that used to prove the swallow now
    // proves the notification.
    //
    // Both halves matter and neither alone is the fix:
    //   - the warning appears, so the failure is no longer silent;
    //   - the graph is still there, so a transient blip does not wipe data the
    //     user can still read. Clearing `graph` on error would also have made
    //     the alert appear, and would have been the worse product choice.
    // Asserted on `data-variant` (emitted by `Alert` at `Alert.tsx:85`) rather
    // than on `role="alert"`: BOTH the warning and the destructive variant are
    // in `ASSERTIVE_VARIANTS` (`Alert.tsx:62`) and so both carry that role, and
    // an assertion that cannot tell the two apart would pass on either branch.
    // Inside `waitFor`: the `toHaveBeenCalledTimes(2)` above only waits until
    // the mock is CALLED, not until React has committed the state its rejection
    // sets. Querying synchronously after it can read the DOM one commit early
    // and fail intermittently.
    await waitFor(() => {
      const alert = document.querySelector('[data-slot="alert"]');
      expect(alert).not.toBeNull();
      expect(alert).toHaveAttribute('data-variant', 'warning');
    });
    expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:3');
  });

  it('shows the load error, and no graph, when the FIRST load fails', async () => {
    // The other side of the branch: with no previously loaded graph there is
    // nothing stale to keep, so the destructive "couldn't load" alert is
    // correct and the warning must not be what renders. This guards against a
    // fix that routes every failure through the stale-data warning.
    graphExportMock.mockRejectedValue(new Error('cold load blew up'));

    await act(async () => {
      renderWithProviders(<Brain />, { initialEntries: ['/?tab=graph'] });
    });

    await waitFor(() => {
      expect(document.querySelector('[data-slot="alert"]')).not.toBeNull();
    });
    expect(document.querySelector('[data-slot="alert"]')).toHaveAttribute(
      'data-variant',
      'destructive'
    );
    expect(document.querySelector('[data-variant="warning"]')).toBeNull();
    expect(screen.queryByTestId('memory-graph')).not.toBeInTheDocument();
  });

  it('still surfaces a failure whose message is empty', async () => {
    // `load()` stores `err.message`, so `new Error('')` lands as `''`. Under a
    // truthiness test (`error ? ...`) that failure renders NOTHING and is
    // silently swallowed — the very defect this PR exists to remove, reachable
    // through a different door. Hence `error !== null` in both branches.
    graphExportMock.mockRejectedValue(new Error(''));

    await act(async () => {
      renderWithProviders(<Brain />, { initialEntries: ['/?tab=graph'] });
    });

    await waitFor(() => {
      const alert = document.querySelector('[data-slot="alert"]');
      expect(alert).not.toBeNull();
      expect(alert).toHaveAttribute('data-variant', 'destructive');
    });
  });

  it('still warns when a REFRESH failure carries an empty message', async () => {
    // The sibling test covers an empty message on the FIRST load, which is the
    // destructive branch. This covers the warning branch, and the two need
    // separate cases: a truthiness regression in only one of them would
    // otherwise slip through on the strength of the other.
    graphExportMock.mockResolvedValueOnce(makeGraph(5)).mockRejectedValue(new Error(''));

    await act(async () => {
      renderWithProviders(<Brain />, { initialEntries: ['/?tab=graph'] });
    });
    await waitFor(() => {
      expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:5');
    });

    await act(async () => {
      window.dispatchEvent(new Event('openhuman:memory-tree-completed'));
    });

    await waitFor(() => {
      const alert = document.querySelector('[data-slot="alert"]');
      expect(alert).not.toBeNull();
      expect(alert).toHaveAttribute('data-variant', 'warning');
    });
    // And the stale graph is still there, as in the non-empty case.
    expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:5');
  });

  it('does not warn when an older load succeeds after a newer one failed', async () => {
    // Two `load()` calls overlap and share `graph`/`error`: the initial one and
    // one started by `memory-tree-completed`. If the NEWER fails while the
    // OLDER is still in flight, and the older then succeeds, the success
    // renders a good graph — and must not leave a "your data is stale" warning
    // attached to data that is not stale.
    //
    // Before this PR the leftover error was invisible, so the race was
    // harmless; the warning is what would have made it a visible false alarm.
    let resolveFirst!: (value: unknown) => void;
    const firstCall = new Promise(resolve => {
      resolveFirst = resolve;
    });
    graphExportMock
      .mockImplementationOnce(() => firstCall)
      .mockImplementationOnce(() => Promise.reject(new Error('newer load failed')));

    await act(async () => {
      renderWithProviders(<Brain />, { initialEntries: ['/?tab=graph'] });
    });

    // The newer load starts and fails while the first is still pending.
    await act(async () => {
      window.dispatchEvent(new Event('openhuman:memory-tree-completed'));
    });

    // Assert the PRECONDITION before resolving anything. Without this the test
    // passes vacuously: if the event never started a second load, resolving the
    // first would render a graph with no warning and the final assertions would
    // be satisfied for entirely the wrong reason.
    await waitFor(() => {
      expect(graphExportMock).toHaveBeenCalledTimes(2);
      expect(document.querySelector('[data-slot="alert"]')).not.toBeNull();
    });

    // Now the older, superseded call finally succeeds.
    await act(async () => {
      resolveFirst(makeGraph(4));
    });

    await waitFor(() => {
      expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:4');
    });
    expect(document.querySelector('[data-variant="warning"]')).toBeNull();
  });
});
