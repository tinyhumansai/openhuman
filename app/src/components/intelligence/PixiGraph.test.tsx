import { act, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { GraphNode } from '../../utils/tauriCommands';
import { PixiGraph } from './PixiGraph';

const mocks = vi.hoisted(() => ({ mountPixiGraph: vi.fn() }));

vi.mock('./pixiGraphRenderer', () => ({
  mountPixiGraph: (...args: unknown[]) => mocks.mountPixiGraph(...args),
}));

const NODES: GraphNode[] = [
  { kind: 'summary', id: 'root', label: 'R', level: 0, parent_id: null },
  { kind: 'chunk', id: 'leaf', label: 'L', parent_id: 'root' },
];

describe('<PixiGraph />', () => {
  beforeEach(() => mocks.mountPixiGraph.mockReset());
  afterEach(() => vi.restoreAllMocks());

  it('mounts the renderer with built graph data', async () => {
    const handle = { resetView: vi.fn(), setTheme: vi.fn(), destroy: vi.fn() };
    mocks.mountPixiGraph.mockResolvedValue(handle);
    const { getByTestId } = render(
      <PixiGraph
        nodes={NODES}
        edges={[]}
        mode="tree"
        dark={false}
        resetSignal={0}
        onHover={vi.fn()}
        onOpen={vi.fn()}
      />
    );
    const host = getByTestId('memory-graph-canvas');
    expect(host).toHaveAttribute('data-render-ready', 'false');
    await waitFor(() => expect(mocks.mountPixiGraph).toHaveBeenCalledTimes(1));
    const [, opts] = mocks.mountPixiGraph.mock.calls[0] as [
      HTMLElement,
      { simNodes: unknown[]; links: unknown[]; onReady: () => void },
    ];
    expect(opts.simNodes).toHaveLength(3); // 2 data nodes + synthetic root hub
    expect(opts.links).toHaveLength(1); // leaf -> root
    act(() => opts.onReady());
    expect(host).toHaveAttribute('data-render-ready', 'true');
  });

  it('destroys the renderer on unmount', async () => {
    const handle = { resetView: vi.fn(), setTheme: vi.fn(), destroy: vi.fn() };
    mocks.mountPixiGraph.mockResolvedValue(handle);
    const { unmount } = render(
      <PixiGraph
        nodes={NODES}
        edges={[]}
        mode="tree"
        dark={false}
        resetSignal={0}
        onHover={vi.fn()}
        onOpen={vi.fn()}
      />
    );
    await waitFor(() => expect(mocks.mountPixiGraph).toHaveBeenCalled());
    unmount();
    await waitFor(() => expect(handle.destroy).toHaveBeenCalled());
  });

  it('ignores renderer readiness after cleanup', async () => {
    const handle = { resetView: vi.fn(), setTheme: vi.fn(), destroy: vi.fn() };
    const onReady = vi.fn();
    mocks.mountPixiGraph.mockResolvedValue(handle);
    const { getByTestId, unmount } = render(
      <PixiGraph
        nodes={NODES}
        edges={[]}
        mode="tree"
        dark={false}
        resetSignal={0}
        onHover={vi.fn()}
        onOpen={vi.fn()}
        onReady={onReady}
      />
    );
    const host = getByTestId('memory-graph-canvas');
    await waitFor(() => expect(mocks.mountPixiGraph).toHaveBeenCalledTimes(1));
    const [, opts] = mocks.mountPixiGraph.mock.calls[0] as [HTMLElement, { onReady: () => void }];

    unmount();
    act(() => opts.onReady());

    expect(host).toHaveAttribute('data-render-ready', 'false');
    expect(onReady).not.toHaveBeenCalled();
  });
});
