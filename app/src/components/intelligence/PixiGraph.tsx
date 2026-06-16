/**
 * Thin React host for the imperative Pixi + d3-force renderer.
 *
 * Mounts the WebGL graph into a div and forwards hover/open back to the
 * parent `MemoryGraph` chrome (footer + preview). Pixi owns all canvas
 * interaction; React only manages the lifecycle. Callbacks are held in
 * refs so changing them never tears down and re-creates the GPU context.
 */
import { useEffect, useRef } from 'react';

import { type GraphEdge, type GraphMode, type GraphNode } from '../../utils/tauriCommands';
import { buildGraph } from './memoryGraphLayout';
import { mountPixiGraph, type PixiGraphHandle } from './pixiGraphRenderer';

interface PixiGraphProps {
  nodes: GraphNode[];
  edges: GraphEdge[];
  mode: GraphMode;
  dark: boolean;
  /** Bump to recentre the view (Reset view button). */
  resetSignal: number;
  onHover: (node: GraphNode | null) => void;
  onOpen: (node: GraphNode) => void;
  /** Called if Pixi fails to initialise at runtime so the parent can
   *  fall back to the SVG renderer. */
  onError?: () => void;
  /** Fired once the layout settles (graph is ready to reveal). */
  onReady?: () => void;
}

export function PixiGraph({
  nodes,
  edges,
  mode,
  dark,
  resetSignal,
  onHover,
  onOpen,
  onError,
  onReady,
}: PixiGraphProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const handleRef = useRef<PixiGraphHandle | null>(null);
  const onHoverRef = useRef(onHover);
  const onOpenRef = useRef(onOpen);
  const onErrorRef = useRef(onError);
  const onReadyRef = useRef(onReady);
  const darkRef = useRef(dark);
  onHoverRef.current = onHover;
  onOpenRef.current = onOpen;
  onErrorRef.current = onError;
  onReadyRef.current = onReady;
  darkRef.current = dark;

  // Mount the renderer once; update in-place when graph data changes.
  const mountedModeRef = useRef<GraphMode | null>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    // Mode change requires full remount (different edge semantics).
    if (handleRef.current && mountedModeRef.current === mode) {
      const { simNodes, links } = buildGraph(nodes, edges, mode);
      handleRef.current.updateGraph(simNodes, links);
      return;
    }

    // First mount or mode flip — full init.
    let cancelled = false;
    handleRef.current?.destroy();
    handleRef.current = null;
    const { simNodes, links } = buildGraph(nodes, edges, mode);
    const pending = mountPixiGraph(host, {
      simNodes,
      links,
      dark: darkRef.current,
      onHover: n => onHoverRef.current(n),
      onOpen: n => onOpenRef.current(n),
      onReady: () => onReadyRef.current?.(),
    })
      .then(handle => {
        if (cancelled) {
          handle.destroy();
          return null;
        }
        handleRef.current = handle;
        mountedModeRef.current = mode;
        return handle;
      })
      .catch(err => {
        console.error('[memory-graph] Pixi init failed; falling back to SVG', err);
        if (!cancelled) onErrorRef.current?.();
        return null;
      });
    return () => {
      cancelled = true;
      handleRef.current = null;
      mountedModeRef.current = null;
      void pending.then(handle => handle?.destroy());
    };
  }, [nodes, edges, mode]);

  useEffect(() => {
    handleRef.current?.setTheme(dark);
  }, [dark]);

  useEffect(() => {
    if (resetSignal > 0) handleRef.current?.resetView();
  }, [resetSignal]);

  return (
    <div
      ref={hostRef}
      data-testid="memory-graph-canvas"
      className="block w-full"
      style={{ height: 'min(640px, calc(100vh - 22rem))' }}
    />
  );
}
