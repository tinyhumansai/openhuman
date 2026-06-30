import { act, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import Brain from '../Brain';

const graphExportMock = vi.hoisted(() => vi.fn());
// Controllable authenticated identity so we can simulate a logout→login cycle
// (userId null → set) and assert the graph reloads (#4149).
const coreAuthRef = vi.hoisted(() => ({ current: 'user-A' as string | null }));

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

vi.mock('../../hooks/useSubconscious', () => ({
  useSubconscious: () => ({
    status: null,
    mode: 'off',
    refresh: vi.fn(),
    triggerTick: vi.fn(),
    setMode: vi.fn(),
  }),
}));

vi.mock('../../components/intelligence/IntelligenceSubconsciousTab', async () => {
  const React = await import('react');
  return { default: () => React.createElement('div', { 'data-testid': 'brain-subconscious' }) };
});
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

// Knowledge & Memory tabs render relocated settings panels — stub each so the
// Brain page's per-tab branches are exercised without their deep dependency trees.
vi.mock('../Intelligence', async () => {
  const React = await import('react');
  return { default: () => React.createElement('div', { 'data-testid': 'brain-intelligence' }) };
});
vi.mock('../../components/settings/panels/MemoryDataPanel', async () => {
  const React = await import('react');
  return { default: () => React.createElement('div', { 'data-testid': 'brain-memory-data' }) };
});
vi.mock('../../components/settings/panels/MemoryDebugPanel', async () => {
  const React = await import('react');
  return { default: () => React.createElement('div', { 'data-testid': 'brain-memory-debug' }) };
});
vi.mock('../../components/settings/panels/AnalysisViewsPanel', async () => {
  const React = await import('react');
  return { default: () => React.createElement('div', { 'data-testid': 'brain-analysis-views' }) };
});
vi.mock('../../components/settings/layout/SettingsLayoutContext', async () => {
  const React = await import('react');
  return {
    SettingsLayoutProvider: ({ children }: { children?: React.ReactNode }) =>
      React.createElement(React.Fragment, null, children),
  };
});

const makeGraph = (n: number) => ({
  nodes: Array.from({ length: n }, (_, i) => ({ id: `n${i}`, kind: 'summary', label: `N${i}` })),
  edges: [],
  content_root_abs: '/tmp/content',
});

describe('Brain page', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    coreAuthRef.current = 'user-A';
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('renders the graph once data is fetched', async () => {
    graphExportMock.mockResolvedValue(makeGraph(3));
    await act(async () => {
      renderWithProviders(<Brain />);
    });
    await waitFor(() => {
      expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:3');
    });
  });

  it('renders empty-state graph when there are no nodes', async () => {
    graphExportMock.mockResolvedValue(makeGraph(0));
    await act(async () => {
      renderWithProviders(<Brain />);
    });
    await waitFor(() => {
      expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:0');
    });
  });

  it('reloads the memory graph from the store when the user re-authenticates (#4149)', async () => {
    // Start signed-out / mid identity-flip: the first fetch resolves empty.
    coreAuthRef.current = null;
    graphExportMock.mockResolvedValue(makeGraph(0));
    let view!: ReturnType<typeof renderWithProviders>;
    await act(async () => {
      view = renderWithProviders(<Brain />);
    });
    await waitFor(() => expect(graphExportMock).toHaveBeenCalledTimes(1));
    expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:0');

    // Re-login: identity becomes available — the graph must re-pull from the
    // persistent store rather than keep the signed-out empty state.
    coreAuthRef.current = 'user-A';
    graphExportMock.mockResolvedValue(makeGraph(5));
    await act(async () => {
      view.rerender(<Brain />);
    });
    await waitFor(() => expect(graphExportMock).toHaveBeenCalledTimes(2));
    await waitFor(() => {
      expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:5');
    });
  });

  it('surfaces an error alert when the fetch fails', async () => {
    graphExportMock.mockRejectedValue(new Error('boom'));
    await act(async () => {
      renderWithProviders(<Brain />);
    });
    await waitFor(() => {
      expect(screen.getByRole('alert')).toBeInTheDocument();
    });
  });

  // The Knowledge & Memory tabs render relocated settings panels inside the
  // two-pane shell; the bespoke tabs share the standard scaffold. Drive each via
  // the `?tab=` query param so every per-tab branch is exercised.
  it.each([
    ['intelligence', 'brain-intelligence'],
    ['memory-data', 'brain-memory-data'],
    ['memory-debug', 'brain-memory-debug'],
    ['analysis-views', 'brain-analysis-views'],
    ['sources', 'brain-sources'],
    ['sync', 'brain-sync'],
    ['subconscious', 'brain-subconscious'],
  ])('renders the %s tab', async (tab, testId) => {
    graphExportMock.mockResolvedValue(makeGraph(0));
    await act(async () => {
      renderWithProviders(<Brain />, { initialEntries: [`/?tab=${tab}`] });
    });
    await waitFor(() => {
      expect(screen.getByTestId(testId)).toBeInTheDocument();
    });
  });
});
