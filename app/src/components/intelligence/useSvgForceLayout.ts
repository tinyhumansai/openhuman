/**
 * React hook that drives the SVG fallback's layout from a Web Worker.
 *
 * When enabled, it spins up {@link svgForceLayout.worker} with the seeded node
 * positions + edges, applies each streamed position frame back onto the same
 * `nodes` array (mutated in place, by index), and calls `onTick` so the SVG
 * repaints — the graph visibly settles instead of freezing. The worker is torn
 * down on data change / unmount / `stop()`.
 *
 * If `Worker` is unavailable (jsdom under test, or a runtime without workers),
 * the hook is a no-op and the caller's synchronous `relaxLayout` fallback runs
 * instead — so behaviour and tests are unchanged off the worker path.
 */
import { useCallback, useEffect, useRef } from 'react';

export interface SvgLayoutNode {
  x: number;
  y: number;
}

interface WorkerTickMessage {
  type: 'tick';
  positions: Float32Array;
  alpha: number;
}

export const WORKER_SUPPORTED = typeof Worker !== 'undefined';

/**
 * @param enabled    only true on the SVG path with worker support
 * @param nodes      the live SimNode array (mutated in place with streamed x/y)
 * @param radii      per-node collision radius, index-aligned with `nodes`
 * @param edges      index pairs [childIdx, parentIdx]
 * @param center     [cx, cy] viewport centre the layout settles around
 * @param alpha      initial sim energy: 1 fresh graph, ~0.3 incremental update
 * @param onTick     stable callback to trigger a repaint (e.g. an imperative apply)
 * @param onSettled  stable callback fired once the sim cools (good time to fit)
 */
export function useSvgForceLayout(
  enabled: boolean,
  nodes: SvgLayoutNode[],
  radii: number[],
  edges: Array<[number, number]>,
  center: readonly [number, number],
  alpha: number,
  onTick: () => void,
  onSettled: () => void
): { drag: (index: number, x: number, y: number, fixed: boolean) => void; stop: () => void } {
  const workerRef = useRef<Worker | null>(null);
  // Session-alive guard, shared by the message handler, the effect cleanup, and
  // stop(); flipping it false makes any in-flight tick/end a no-op.
  const aliveRef = useRef(false);

  useEffect(() => {
    if (!enabled || !WORKER_SUPPORTED || nodes.length === 0) return;

    const worker = new Worker(new URL('./svgForceLayout.worker.ts', import.meta.url), {
      type: 'module',
    });
    workerRef.current = worker;
    aliveRef.current = true;

    worker.onmessage = (e: MessageEvent) => {
      // aliveRef is flipped false by the cleanup AND by stop(), so a late
      // tick/end from a just-stopped worker can't mutate nodes or fit.
      if (!aliveRef.current) return;
      const msg = e.data as WorkerTickMessage | { type: 'end' };
      if (msg.type === 'tick') {
        const pos = msg.positions;
        const n = Math.min(nodes.length, pos.length >> 1);
        for (let i = 0; i < n; i++) {
          nodes[i].x = pos[i * 2];
          nodes[i].y = pos[i * 2 + 1];
        }
        onTick();
      } else if (msg.type === 'end') {
        onSettled();
      }
    };

    worker.postMessage({
      type: 'init',
      nodes: nodes.map((node, i) => ({ x: node.x, y: node.y, r: radii[i] ?? 6 })),
      links: edges.map(([a, b]) => ({ source: a, target: b })),
      cx: center[0],
      cy: center[1],
      alpha,
    });

    return () => {
      aliveRef.current = false;
      worker.postMessage({ type: 'stop' });
      worker.terminate();
      workerRef.current = null;
    };
  }, [enabled, nodes, radii, edges, center, alpha, onTick, onSettled]);

  const drag = useCallback((index: number, x: number, y: number, fixed: boolean) => {
    workerRef.current?.postMessage({ type: 'drag', index, x, y, fixed });
  }, []);
  const stop = useCallback(() => {
    aliveRef.current = false; // ignore any in-flight tick/end from this worker
    workerRef.current?.postMessage({ type: 'stop' });
    workerRef.current?.terminate();
    workerRef.current = null;
  }, []);

  return { drag, stop };
}
