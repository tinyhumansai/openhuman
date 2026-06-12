/**
 * Exercises the worker-backed SVG path of MemoryGraph, which can't run under
 * the default test env (no real Worker). A mocked Worker lets us drive the
 * streamed-position apply, settle-time fit, drag-freeze, and progressive mount.
 *
 * `WORKER_SUPPORTED` is captured at module load, so Worker must be stubbed
 * before MemoryGraph is imported — hence the dynamic import in beforeAll.
 */
import { configureStore } from '@reduxjs/toolkit';
import { act, fireEvent, render } from '@testing-library/react';
import type { PropsWithChildren } from 'react';
import { Provider } from 'react-redux';
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import themeReducer from '../../store/themeSlice';
import type { GraphNode } from '../../utils/tauriCommands';

function ReduxWrapper({ children }: PropsWithChildren) {
  const store = configureStore({ reducer: { theme: themeReducer } });
  return <Provider store={store}>{children}</Provider>;
}

vi.mock('../../utils/openUrl', () => ({ openUrl: vi.fn() }));
vi.mock('../../utils/tauriCommands/workspacePaths', () => ({
  openWorkspacePath: vi.fn(),
  previewWorkspaceText: vi.fn(),
}));

class MockWorker {
  static instances: MockWorker[] = [];
  onmessage: ((e: { data: unknown }) => void) | null = null;
  posted: Array<Record<string, unknown>> = [];
  terminated = false;
  constructor() {
    MockWorker.instances.push(this);
  }
  postMessage(m: Record<string, unknown>) {
    this.posted.push(m);
  }
  terminate() {
    this.terminated = true;
  }
  emit(data: unknown) {
    this.onmessage?.({ data });
  }
}

const last = () => MockWorker.instances[MockWorker.instances.length - 1];

let MemoryGraph: typeof import('./MemoryGraph').MemoryGraph;

beforeAll(async () => {
  vi.stubGlobal('Worker', MockWorker as unknown as typeof Worker);
  // Run rAF synchronously so imperative position writes + the mount ramp are
  // deterministic within a test tick.
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
    cb(0);
    return 1;
  });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  ({ MemoryGraph } = await import('./MemoryGraph'));
});

function treeNodes(leaves: number): GraphNode[] {
  const nodes: GraphNode[] = [{ kind: 'source', id: 's', label: 's', parent_id: null }];
  for (let i = 0; i < leaves; i++) {
    nodes.push({ kind: 'chunk', id: `c${i}`, label: `c${i}`, parent_id: 's' });
  }
  return nodes;
}

/** Give the SVG a usable CTM so pan/drag math doesn't no-op under jsdom. */
function withCTM(svg: Element) {
  (svg as unknown as { getScreenCTM: () => unknown }).getScreenCTM = () => ({
    inverse: () => ({ a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 }),
  });
}

describe('<MemoryGraph /> worker-backed SVG path', () => {
  beforeEach(() => {
    MockWorker.instances = [];
  });

  it('spins up a worker and applies streamed positions imperatively', () => {
    const { container } = render(<MemoryGraph nodes={treeNodes(3)} edges={[]} mode="tree" />, {
      wrapper: ReduxWrapper,
    });
    const w = last();
    expect(w).toBeTruthy();
    expect(w.posted[0].type).toBe('init');
    expect(w.posted[0].alpha).toBe(1); // fresh graph → full reheat

    act(() =>
      w.emit({
        type: 'tick',
        positions: new Float32Array([11, 22, 33, 44, 55, 66, 77, 88]),
        alpha: 0.5,
      })
    );
    const circles = container.querySelectorAll('circle');
    expect(circles.length).toBeGreaterThan(0);
    expect(circles[0].getAttribute('cx')).toBe('11');
    expect(circles[0].getAttribute('cy')).toBe('22');
  });

  it('frames the graph on settle (end → fit transform)', () => {
    const { container } = render(<MemoryGraph nodes={treeNodes(3)} edges={[]} mode="tree" />, {
      wrapper: ReduxWrapper,
    });
    const w = last();
    act(() =>
      w.emit({
        type: 'tick',
        positions: new Float32Array([0, 0, 200, 0, 0, 200, 200, 200]),
        alpha: 0.1,
      })
    );
    act(() => w.emit({ type: 'end' }));
    const g = container.querySelector('[data-testid="memory-graph-svg"] g');
    expect(g?.getAttribute('transform')).toMatch(/translate\(.*\) scale\(/);
  });

  it('freezes the worker on a drag, but NOT on a plain click', () => {
    const { container } = render(<MemoryGraph nodes={treeNodes(3)} edges={[]} mode="tree" />, {
      wrapper: ReduxWrapper,
    });
    const svg = container.querySelector('[data-testid="memory-graph-svg"]') as Element;
    withCTM(svg);
    const w = last();

    // Plain click (down + up, no move) must not stop layout.
    fireEvent.pointerDown(svg, { clientX: 10, clientY: 10 });
    fireEvent.pointerUp(svg, { clientX: 10, clientY: 10 });
    expect(w.terminated).toBe(false);

    // Drag (down + move) hands the camera over → stop().
    fireEvent.pointerDown(svg, { clientX: 10, clientY: 10 });
    fireEvent.pointerMove(svg, { clientX: 80, clientY: 80 });
    expect(w.terminated).toBe(true);
  });

  it('reheats gently and carries positions over on an incremental update', () => {
    const { rerender } = render(<MemoryGraph nodes={treeNodes(3)} edges={[]} mode="tree" />, {
      wrapper: ReduxWrapper,
    });
    const first = last();
    act(() =>
      first.emit({
        type: 'tick',
        positions: new Float32Array([1, 1, 2, 2, 3, 3, 4, 4]),
        alpha: 0.5,
      })
    );
    // Add a node → same instance, new props → new worker session.
    rerender(<MemoryGraph nodes={treeNodes(4)} edges={[]} mode="tree" />);
    const second = last();
    expect(second).not.toBe(first);
    expect(second.posted[0].type).toBe('init');
    expect(second.posted[0].alpha).toBe(0.3); // survivors present → gentle reheat
  });

  it('mounts a large graph progressively without throwing', () => {
    expect(() =>
      render(<MemoryGraph nodes={treeNodes(900)} edges={[]} mode="tree" />, {
        wrapper: ReduxWrapper,
      })
    ).not.toThrow();
  });

  it('handles node drag, wheel zoom, and reset view', () => {
    const { container, getByTestId } = render(
      <MemoryGraph nodes={treeNodes(3)} edges={[]} mode="tree" />,
      { wrapper: ReduxWrapper }
    );
    const svg = container.querySelector('[data-testid="memory-graph-svg"]') as Element;
    withCTM(svg);
    const w = last();

    // Drag a node → freezes layout and repositions the node.
    const node = container.querySelector('[data-testid="memory-graph-node-c0"]') as Element;
    fireEvent.pointerDown(node, { clientX: 20, clientY: 20 });
    fireEvent.pointerMove(svg, { clientX: 90, clientY: 90 });
    fireEvent.pointerUp(svg, { clientX: 90, clientY: 90 });
    expect(w.terminated).toBe(true);

    // Wheel zoom is a real interaction (no throw).
    expect(() => fireEvent.wheel(svg, { deltaY: -120, clientX: 50, clientY: 50 })).not.toThrow();

    // Reset view re-frames.
    expect(() => fireEvent.click(getByTestId('memory-graph-reset-view'))).not.toThrow();
  });
});
