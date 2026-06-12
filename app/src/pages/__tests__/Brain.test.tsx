/**
 * Tests for the Brain page orchestration: it shows the loading overlay, then
 * reveals the memory graph once data is fetched, the layout has settled
 * (mocked MemoryGraph fires onReady), and the minimum animation window passes.
 * Heavy children (the Pixi/SVG graph and the Lottie player) and the RPC are
 * mocked so this stays a fast unit test of the gate logic.
 */
import { act, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Brain from '../Brain';

const graphExportMock = vi.hoisted(() => vi.fn());

vi.mock('../../utils/tauriCommands', () => ({ memoryTreeGraphExport: graphExportMock }));

// Mocked graph: fires onReady on mount to simulate the layout settling.
vi.mock('../../components/intelligence/MemoryGraph', async () => {
  const React = await import('react');
  return {
    MemoryGraph: ({ nodes, onReady }: { nodes: unknown[]; onReady?: () => void }) => {
      React.useEffect(() => {
        onReady?.();
      }, [onReady]);
      return React.createElement('div', { 'data-testid': 'memory-graph' }, `nodes:${nodes.length}`);
    },
  };
});

vi.mock('../../components/LottieAnimation', async () => {
  const React = await import('react');
  return {
    default: ({ src }: { src: string }) =>
      React.createElement('div', { 'data-testid': 'brain-lottie', 'data-src': src }),
  };
});

// The memory workspace panels own their own polling/RPCs — stub them so this
// stays an isolated test of the Brain page's loading/reveal orchestration.
vi.mock('../../components/intelligence/MemoryControls', () => ({ MemoryControls: () => null }));
vi.mock('../../components/intelligence/MemoryTreeStatusPanel', () => ({
  MemoryTreeStatusPanel: () => null,
}));
vi.mock('../../components/intelligence/MemorySourcesRegistry', () => ({
  MemorySourcesRegistry: () => null,
}));
vi.mock('../../components/intelligence/Toast', () => ({ ToastContainer: () => null }));

const makeGraph = (n: number) => ({
  nodes: Array.from({ length: n }, (_, i) => ({ id: `n${i}`, kind: 'summary', label: `N${i}` })),
  edges: [],
  content_root_abs: '/tmp/content',
});

describe('Brain page', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    // jsdom has no matchMedia — default to "motion allowed".
    window.matchMedia = vi
      .fn()
      .mockReturnValue({
        matches: false,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      }) as unknown as typeof window.matchMedia;
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('shows the loading overlay first, then reveals the graph once ready', async () => {
    graphExportMock.mockResolvedValue(makeGraph(3));
    render(<Brain />);

    // Overlay (with the Lottie flourish) is visible immediately.
    expect(screen.getByTestId('brain-loading')).toBeInTheDocument();
    expect(screen.getByTestId('brain-lottie')).toHaveAttribute(
      'data-src',
      '/lottie/brain-collecting.json'
    );

    // Flush the fetch + the minimum-display timer.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1300);
    });

    expect(screen.queryByTestId('brain-loading')).toBeNull();
    expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:3');
  });

  it('reveals immediately (empty state) when there is no memory yet', async () => {
    graphExportMock.mockResolvedValue(makeGraph(0));
    render(<Brain />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1300);
    });

    expect(screen.queryByTestId('brain-loading')).toBeNull();
    expect(screen.getByTestId('memory-graph')).toHaveTextContent('nodes:0');
  });

  it('uses the static knowledge-node glyph (no Lottie) under reduced motion', async () => {
    // Reduced motion → the floating Lottie is replaced by the still glyph.
    window.matchMedia = vi
      .fn()
      .mockReturnValue({
        matches: true,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      }) as unknown as typeof window.matchMedia;
    graphExportMock.mockResolvedValue(makeGraph(2));
    render(<Brain />);

    expect(screen.getByTestId('brain-loading')).toBeInTheDocument();
    expect(screen.getByTestId('brain-glyph')).toBeInTheDocument();
    expect(screen.queryByTestId('brain-lottie')).toBeNull();
  });

  it('surfaces an error and dismisses the overlay when the fetch fails', async () => {
    graphExportMock.mockRejectedValue(new Error('boom'));
    render(<Brain />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(50);
    });

    expect(screen.queryByTestId('brain-loading')).toBeNull();
    expect(screen.getByRole('alert')).toBeInTheDocument();
  });
});
