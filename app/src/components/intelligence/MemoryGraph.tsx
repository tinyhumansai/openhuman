/**
 * Obsidian-style force-directed graph view for the memory tree.
 *
 * Two modes:
 *   - `tree`     — sealed summary nodes connected by parent→child
 *   - `contacts` — raw chunks linked to person entities they mention
 *
 * Layout: a tiny barycentric force simulation
 *   - parent → child links pull connected nodes together
 *   - all-pairs Coulomb repulsion pushes overlapping nodes apart
 *   - centring force keeps the cloud anchored in the viewport
 *
 * Colour: in tree mode each level lights up in its own hue (mirroring the
 * Obsidian `path:L{n}` groups) with a soft glow on summary nodes; leaves
 * stay a quiet slate.
 *
 * Interaction: drag a node to reposition it, drag the background to pan,
 * scroll to zoom, and "Reset view" recentres. Click a summary node →
 * opens the matching `.md` file through the shared workspace path
 * command (skipped when the pointer was dragging). This keeps Memory
 * graph file actions on the same guarded contract as chat workspace links.
 *
 * Rendering: where WebGL is available we use a Pixi.js + d3-force canvas
 * ({@link PixiGraph}) — the same stack Obsidian's graph runs on, smooth
 * well past the 1000-node cap. Without WebGL (e.g. jsdom under test) it
 * falls back to a deterministic pure-SVG renderer with the same colours,
 * interactions and click/preview behaviour.
 */
import {
  type PointerEvent as ReactPointerEvent,
  type WheelEvent as ReactWheelEvent,
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { useAppSelector } from '../../store/hooks';
import { resolveTheme, type ThemeMode } from '../../store/themeSlice';
import { type GraphEdge, type GraphMode, type GraphNode } from '../../utils/tauriCommands';
import { openWorkspacePath, previewWorkspaceText } from '../../utils/tauriCommands/workspacePaths';
import Button from '../ui/Button';
import {
  CONTACT_COLOR,
  LEAF_COLOR,
  levelColor,
  nodeColor,
  nodeRadius,
  SOURCE_COLOR,
  VIEWPORT_H,
  VIEWPORT_W,
  ZOOM_MAX,
  ZOOM_MIN,
} from './memoryGraphLayout';
import { summaryWorkspacePath } from './memoryWorkspacePaths';
import { PixiGraph } from './PixiGraph';
import { seedSvgLayout } from './seedSvgLayout';
import { useSvgForceLayout, WORKER_SUPPORTED } from './useSvgForceLayout';

/** Use WebGL (Pixi) in production; fall back to SVG in test (jsdom). */
const HAS_WEBGL =
  typeof document !== 'undefined' &&
  typeof document.createElement === 'function' &&
  (() => {
    try {
      const c = document.createElement('canvas');
      return !!(c.getContext('webgl2') || c.getContext('webgl'));
    } catch {
      return false;
    }
  })();

interface SimNode extends GraphNode {
  x: number;
  y: number;
  vx: number;
  vy: number;
}

interface SimState {
  sim: SimNode[];
  edges: Array<[number, number]>;
  radii: number[];
  alpha: number;
}

// Stable empties so the worker-layout effect's deps don't change every render
// when there's no graph yet.
const NO_NODES: SimNode[] = [];
const NO_RADII: number[] = [];
const NO_EDGES: Array<[number, number]> = [];
// Stable centre the SVG worker layout settles around (matches the viewBox).
const SVG_CENTER: readonly [number, number] = [VIEWPORT_W / 2, VIEWPORT_H / 2];

interface MemoryGraphProps {
  /** Pre-fetched summary / chunk / contact nodes. */
  nodes: GraphNode[];
  /** Explicit edges (only used in contacts mode). */
  edges: GraphEdge[];
  /** Which graph this is — drives colour palette + click behaviour. */
  mode: GraphMode;
  /** Optional override for the empty-state message. */
  emptyHint?: string;
  /**
   * Fired exactly once when the graph's force layout first settles (SVG
   * worker `end`, the synchronous relax fallback, or the Pixi sim cooling).
   * A loading overlay (e.g. the Brain page) waits on this to reveal the graph.
   */
  onReady?: () => void;
}

interface SummaryPreviewState {
  path: string;
  contents: string;
  truncated: boolean;
  error: string | null;
}

/**
 * Map a pointer's client coords into the SVG's viewBox coordinate space
 * (SVG fallback only). Returns null without a live CTM (e.g. jsdom) so the
 * pan/zoom handlers degrade to no-ops under test.
 */
function clientToViewBox(
  svg: SVGSVGElement | null,
  clientX: number,
  clientY: number
): { x: number; y: number } | null {
  if (!svg || typeof svg.getScreenCTM !== 'function') return null;
  const ctm = svg.getScreenCTM();
  if (!ctm) return null;
  const inv = ctm.inverse();
  return {
    x: inv.a * clientX + inv.c * clientY + inv.e,
    y: inv.b * clientX + inv.d * clientY + inv.f,
  };
}

/**
 * Run the force simulation for `iterations` ticks. Mutates positions in
 * place so we can re-use the same buffer across renders.
 */
function relaxLayout(nodes: SimNode[], edges: Array<[number, number]>, iterations = 220): void {
  const REPULSION = 1800;
  const SPRING_K = 0.04;
  const SPRING_LEN = 60;
  const CENTER_K = 0.0025;
  const FRICTION = 0.85;
  const cx = VIEWPORT_W / 2;
  const cy = VIEWPORT_H / 2;

  for (let iter = 0; iter < iterations; iter++) {
    for (let i = 0; i < nodes.length; i++) {
      for (let j = i + 1; j < nodes.length; j++) {
        const a = nodes[i];
        const b = nodes[j];
        const dx = a.x - b.x;
        const dy = a.y - b.y;
        const dist2 = dx * dx + dy * dy + 0.01;
        const force = REPULSION / dist2;
        const dist = Math.sqrt(dist2);
        const fx = (dx / dist) * force;
        const fy = (dy / dist) * force;
        a.vx += fx;
        a.vy += fy;
        b.vx -= fx;
        b.vy -= fy;
      }
    }
    for (const [ai, bi] of edges) {
      const a = nodes[ai];
      const b = nodes[bi];
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const dist = Math.sqrt(dx * dx + dy * dy) + 0.01;
      const delta = dist - SPRING_LEN;
      const fx = (dx / dist) * delta * SPRING_K;
      const fy = (dy / dist) * delta * SPRING_K;
      a.vx += fx;
      a.vy += fy;
      b.vx -= fx;
      b.vy -= fy;
    }
    for (const n of nodes) {
      n.vx += (cx - n.x) * CENTER_K;
      n.vy += (cy - n.y) * CENTER_K;
      n.vx *= FRICTION;
      n.vy *= FRICTION;
      n.x += n.vx;
      n.y += n.vy;
    }
  }
}

export function MemoryGraph({ nodes, edges, mode, emptyHint, onReady }: MemoryGraphProps) {
  const { t } = useT();
  const themeMode = useAppSelector(state => state.theme?.mode ?? 'system') as ThemeMode;
  const isDark = resolveTheme(themeMode) === 'dark';
  const [hovered, setHovered] = useState<GraphNode | null>(null);

  // Fire `onReady` at most once across this component's lifetime. The latest
  // callback is held in a ref so `fireReady` stays stable (the SVG layout hook
  // depends on a stable `onSettled`, and the guard prevents refires on reheat).
  const onReadyRef = useRef(onReady);
  onReadyRef.current = onReady;
  const readyFiredRef = useRef(false);
  const fireReady = useCallback(() => {
    if (readyFiredRef.current) return;
    readyFiredRef.current = true;
    console.debug('[memory-graph] layout settled → onReady');
    onReadyRef.current?.();
  }, []);
  const [preview, setPreview] = useState<SummaryPreviewState | null>(null);
  const [previewingPath, setPreviewingPath] = useState<string | null>(null);
  const svgRef = useRef<SVGSVGElement | null>(null);

  // Pan / zoom transform applied to the graph group, plus the live drag
  // state. Node positions live in the memoised `sim` buffer and are
  // mutated in place during a node drag; `bumpTick` forces a re-render so
  // the moved node repaints without re-running the physics.
  const [view, setView] = useState({ tx: 0, ty: 0, scale: 1 });
  const [, bumpTick] = useReducer((c: number) => c + 1, 0);
  const [grabbing, setGrabbing] = useState(false);
  // Bumped by "Reset view" — the Pixi renderer watches it to recentre.
  const [resetSignal, bumpReset] = useReducer((c: number) => c + 1, 0);
  // Flips true if Pixi fails to init at runtime → fall back to SVG even
  // though supportsWebGL() was true at module load.
  const [pixiFailed, setPixiFailed] = useState(false);
  const useWebGL = HAS_WEBGL && !pixiFailed;
  const dragRef = useRef<
    | { kind: 'node'; node: SimNode; dx: number; dy: number }
    | { kind: 'pan'; vbStartX: number; vbStartY: number; tx0: number; ty0: number }
    | null
  >(null);
  // True once the pointer moved during the current gesture — guards the
  // node click so a drag doesn't also open the summary file.
  const movedRef = useRef(false);
  // Halts the SVG layout worker once the user grabs a node/background, so its
  // streamed positions stop fighting the manual drag. Set after the hook below.
  const stopLayoutRef = useRef<() => void>(() => {});
  // Set once the user grabs the camera, so the settle-time auto-fit doesn't
  // yank the view out from under them.
  const userInteractedRef = useRef(false);
  // Re-frame the SVG graph from "Reset view" (set after fitToView below).
  const fitRef = useRef<() => void>(() => {});
  // Holds the current sim across renders; during the next build it still points
  // at the OUTGOING sim, whose nodes carry the latest live coordinates (the
  // worker / a drag mutate them in place) — read for position carry-over.
  const liveSimRef = useRef<SimState | null>(null);

  const clientToGraph = useCallback(
    (clientX: number, clientY: number) => {
      const vb = clientToViewBox(svgRef.current, clientX, clientY);
      if (!vb) return null;
      return { x: (vb.x - view.tx) / view.scale, y: (vb.y - view.ty) / view.scale };
    },
    [view]
  );

  const onNodePointerDown = useCallback(
    (e: ReactPointerEvent, n: SimNode) => {
      // Stop the background pan from also starting on this pointer down.
      e.stopPropagation();
      movedRef.current = false;
      const g = clientToGraph(e.clientX, e.clientY);
      if (!g) return;
      (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
      dragRef.current = { kind: 'node', node: n, dx: g.x - n.x, dy: g.y - n.y };
      setGrabbing(true);
    },
    [clientToGraph]
  );

  const onBackgroundPointerDown = useCallback(
    (e: ReactPointerEvent) => {
      movedRef.current = false;
      const vb = clientToViewBox(svgRef.current, e.clientX, e.clientY);
      if (!vb) return;
      (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
      dragRef.current = { kind: 'pan', vbStartX: vb.x, vbStartY: vb.y, tx0: view.tx, ty0: view.ty };
      setGrabbing(true);
    },
    [view]
  );

  const onPointerMove = useCallback(
    (e: ReactPointerEvent) => {
      const d = dragRef.current;
      if (!d) return;
      // On the first real movement (not a plain click), hand the camera to the
      // user: freeze the worker layout and suppress the settle-time auto-fit.
      if (!movedRef.current) {
        stopLayoutRef.current();
        userInteractedRef.current = true;
      }
      if (d.kind === 'node') {
        const g = clientToGraph(e.clientX, e.clientY);
        if (!g) return;
        d.node.x = g.x - d.dx;
        d.node.y = g.y - d.dy;
        movedRef.current = true;
        bumpTick();
      } else {
        const vb = clientToViewBox(svgRef.current, e.clientX, e.clientY);
        if (!vb) return;
        movedRef.current = true;
        setView(v => ({ ...v, tx: d.tx0 + (vb.x - d.vbStartX), ty: d.ty0 + (vb.y - d.vbStartY) }));
      }
    },
    [clientToGraph]
  );

  const endDrag = useCallback(() => {
    dragRef.current = null;
    setGrabbing(false);
  }, []);

  const onWheelZoom = useCallback((e: ReactWheelEvent) => {
    const vb = clientToViewBox(svgRef.current, e.clientX, e.clientY);
    if (!vb) return;
    userInteractedRef.current = true;
    setView(v => {
      const factor = Math.exp(-e.deltaY * 0.0015);
      const scale = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, v.scale * factor));
      // Keep the graph point under the cursor fixed while zooming.
      const gx = (vb.x - v.tx) / v.scale;
      const gy = (vb.y - v.ty) / v.scale;
      return { scale, tx: vb.x - gx * scale, ty: vb.y - gy * scale };
    });
  }, []);

  const resetView = useCallback(() => {
    // SVG re-frames the whole graph; the Pixi canvas listens on the reset
    // signal. Both are triggered so the button works in either path.
    userInteractedRef.current = false;
    fitRef.current();
    bumpReset();
  }, []);

  const openSummary = useCallback(async (node: GraphNode) => {
    const path = summaryWorkspacePath(node);
    if (!path) return;
    console.debug('[memory-graph] open workspace path=%s', path);
    try {
      await openWorkspacePath(path);
    } catch (err) {
      console.error('[memory-graph] openWorkspacePath failed', err);
    }
  }, []);

  const previewSummary = useCallback(async (node: GraphNode) => {
    const path = summaryWorkspacePath(node);
    if (!path) return;
    setPreviewingPath(path);
    try {
      const next = await previewWorkspaceText(path);
      setPreview({ path, contents: next.contents, truncated: next.truncated, error: null });
    } catch (err) {
      console.error('[memory-graph] previewWorkspaceText failed', err);
      setPreview({
        path,
        contents: '',
        truncated: false,
        error: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setPreviewingPath(null);
    }
  }, []);

  // Build edges + seed positions, carrying over the last positions of any
  // surviving node so a live data update doesn't reshuffle the whole graph
  // (seedSvgLayout). The O(n²) relax only runs as the no-worker fallback (test
  // env); the worker settles otherwise; the Pixi path runs its own sim.
  const sim = useMemo<SimState | null>(() => {
    if (!nodes || nodes.length === 0) return null;
    // Snapshot the OUTGOING graph's live coordinates so survivors carry over
    // from where they actually are now — not a stale init/settle snapshot —
    // even mid-settle or after a drag.
    const prev = liveSimRef.current;
    const prevPos = new Map<string, { x: number; y: number }>();
    if (prev) for (const n of prev.sim) prevPos.set(n.id, { x: n.x, y: n.y });
    const seed = seedSvgLayout(nodes, edges, mode, prevPos);
    const sim: SimNode[] = nodes.map((n, i) => ({
      ...n,
      x: seed.positions[i].x,
      y: seed.positions[i].y,
      vx: 0,
      vy: 0,
    }));
    const radii = sim.map(n => nodeRadius(n));
    if (!useWebGL && !WORKER_SUPPORTED) relaxLayout(sim, seed.edges);
    return { sim, edges: seed.edges, radii, alpha: seed.reheatAlpha };
  }, [nodes, edges, mode, useWebGL]);
  // Becomes the "previous" sim on the next build (above).
  liveSimRef.current = sim;

  // Element refs for imperative position updates: while the worker streams
  // positions we write cx/cy (and line endpoints) straight to the DOM instead
  // of re-rendering up to 10k elements through React every frame.
  const circleEls = useRef<(SVGCircleElement | null)[]>([]);
  const lineEls = useRef<(SVGLineElement | null)[]>([]);

  // Progressive DOM mount for the SVG path: reveal nodes in per-frame batches
  // so a large graph never blocks building thousands of elements in one commit.
  // WebGL draws to one canvas, so it shows everything at once.
  const FIRST_BATCH = 800;
  const [svgVisible, setSvgVisible] = useState(() =>
    sim ? Math.min(sim.sim.length, FIRST_BATCH) : 0
  );
  // Reset the reveal window + element refs during render when the graph data
  // changes (the recommended alternative to setState-in-effect).
  const simIdRef = useRef(sim);
  if (simIdRef.current !== sim) {
    simIdRef.current = sim;
    circleEls.current = [];
    lineEls.current = [];
    setSvgVisible(sim ? Math.min(sim.sim.length, FIRST_BATCH) : 0);
  }

  // Latest visible count read by the stable imperative applier without
  // re-subscribing the worker every render.
  const latestVisibleRef = useRef(svgVisible);
  latestVisibleRef.current = svgVisible;

  // Write current positions straight to the mounted SVG elements.
  const applyPositions = useCallback(() => {
    const s = liveSimRef.current;
    if (!s) return;
    const vis = latestVisibleRef.current;
    const ns = s.sim;
    for (let i = 0; i < vis && i < ns.length; i++) {
      const el = circleEls.current[i];
      if (el) {
        el.setAttribute('cx', String(ns[i].x));
        el.setAttribute('cy', String(ns[i].y));
      }
    }
    for (let e = 0; e < s.edges.length; e++) {
      const [ai, bi] = s.edges[e];
      if (ai >= vis || bi >= vis) continue;
      const el = lineEls.current[e];
      if (el) {
        el.setAttribute('x1', String(ns[ai].x));
        el.setAttribute('y1', String(ns[ai].y));
        el.setAttribute('x2', String(ns[bi].x));
        el.setAttribute('y2', String(ns[bi].y));
      }
    }
  }, []);

  // Frame the whole cloud in the viewport. d3-force spreads a large graph far
  // past the viewBox, so without this most nodes sit off-screen. Committed to
  // `view` state (single source) so pan/zoom keep working; called once the
  // worker settles, unless the user already grabbed the camera.
  const fitToView = useCallback(() => {
    const s = liveSimRef.current;
    if (!s || userInteractedRef.current) return;
    const ns = s.sim;
    if (ns.length === 0) return;
    // Center on the root node at a fixed comfortable zoom.
    const root = ns.find(n => n.kind === 'root');
    const cx = root?.x ?? 0;
    const cy = root?.y ?? 0;
    const scale = 0.17;
    setView({ scale, tx: VIEWPORT_W / 2 - cx * scale, ty: VIEWPORT_H / 2 - cy * scale });
  }, []);
  fitRef.current = fitToView;

  // Coalesce worker ticks to one DOM write per frame.
  const applyPendingRef = useRef(false);
  const scheduleApply = useCallback(() => {
    if (applyPendingRef.current) return;
    applyPendingRef.current = true;
    const run = () => {
      applyPendingRef.current = false;
      applyPositions();
    };
    if (typeof window.requestAnimationFrame === 'function') window.requestAnimationFrame(run);
    else run();
  }, [applyPositions]);

  // Frame the graph then signal readiness once the worker layout cools.
  const onSvgSettled = useCallback(() => {
    fitToView();
    fireReady();
  }, [fitToView, fireReady]);

  // SVG fallback layout runs in a worker (off the main thread); positions
  // stream back and are applied imperatively. No-op on WebGL and where workers
  // are unavailable (the synchronous relaxLayout above covers that case).
  const svgLayout = useSvgForceLayout(
    !useWebGL && !!sim,
    sim?.sim ?? NO_NODES,
    sim?.radii ?? NO_RADII,
    sim?.edges ?? NO_EDGES,
    SVG_CENTER,
    sim?.alpha ?? 1,
    scheduleApply,
    onSvgSettled
  );
  stopLayoutRef.current = svgLayout.stop;

  // Synchronous-layout path (no WebGL, no Worker — e.g. jsdom under test):
  // `relaxLayout` already ran inside the `sim` memo, so the graph is laid out
  // as soon as `sim` exists. Signal readiness on the next tick.
  useEffect(() => {
    if (useWebGL || WORKER_SUPPORTED || !sim) return;
    fireReady();
  }, [useWebGL, sim, fireReady]);

  // Ramp the rest in per-frame batches (setState only inside the rAF callback).
  useEffect(() => {
    if (useWebGL || !sim) return;
    const total = sim.sim.length;
    if (total <= FIRST_BATCH || typeof window.requestAnimationFrame !== 'function') return;
    let raf = 0;
    const step = () => {
      setSvgVisible(c => {
        const next = Math.min(total, c + 1200);
        if (next < total) raf = window.requestAnimationFrame(step);
        return next;
      });
    };
    raf = window.requestAnimationFrame(step);
    return () => window.cancelAnimationFrame(raf);
  }, [useWebGL, sim]);

  if (nodes.length === 0) {
    return (
      <div
        className="flex h-[640px] items-center justify-center rounded-lg border border-line-subtle bg-surface-muted/40 text-sm text-content-muted"
        data-testid="memory-graph-empty">
        {emptyHint ?? (mode === 'contacts' ? t('graph.noContactMentions') : t('graph.noMemory'))}
      </div>
    );
  }

  if (!sim) return null;

  // Distinct legend rows for the active mode. Tree mode lists the levels
  // actually present (each lit in its own colour) plus a leaf row when
  // chunks are shown.
  const legend =
    mode === 'tree'
      ? [
          ...(nodes.some(n => n.kind === 'source')
            ? [{ label: t('graph.source', 'Source'), color: SOURCE_COLOR }]
            : []),
          ...Array.from(new Set(nodes.filter(n => n.kind === 'summary').map(n => n.level ?? 0)))
            .sort((a, b) => a - b)
            .map(lvl => ({ label: `L${lvl}`, color: levelColor(lvl) })),
          ...(nodes.some(n => n.kind === 'chunk')
            ? [{ label: t('graph.document'), color: LEAF_COLOR }]
            : []),
        ]
      : [
          { label: t('graph.document'), color: LEAF_COLOR },
          { label: t('graph.contact'), color: CONTACT_COLOR },
        ];
  const hoveredSummaryPath = hovered?.kind === 'summary' ? summaryWorkspacePath(hovered) : null;

  return (
    <div
      className="memory-graph rounded-lg border border-line-subtle bg-surface"
      onMouseLeave={() => setHovered(null)}>
      <div className="flex items-center justify-between gap-4 border-b border-line-subtle px-4 py-2">
        <div className="flex items-center gap-3 text-xs text-content-muted">
          <span>
            {nodes.length} {t('graph.nodes')}
          </span>
          <span className="text-content-faint dark:text-neutral-600">·</span>
          <span>
            {sim.edges.length}{' '}
            {mode === 'tree' ? t('graph.parentChild') : t('graph.documentContact')}{' '}
            {sim.edges.length === 1 ? t('graph.link') : t('graph.links')}
          </span>
        </div>
        <div className="flex items-center gap-3">
          {legend.map(item => (
            <span
              key={item.label}
              className="flex items-center gap-1.5 text-xs text-content-secondary">
              <span
                className="inline-block h-2.5 w-2.5 rounded-full"
                style={{ backgroundColor: item.color }}
              />
              {item.label}
            </span>
          ))}
          <Button
            variant="secondary"
            size="xs"
            onClick={resetView}
            data-testid="memory-graph-reset-view"
            className="text-[11px] shadow-sm">
            {t('graph.resetView')}
          </Button>
        </div>
      </div>
      {useWebGL ? (
        <PixiGraph
          nodes={nodes}
          edges={edges}
          mode={mode}
          dark={
            typeof document !== 'undefined' && document.documentElement.classList.contains('dark')
          }
          resetSignal={resetSignal}
          onHover={setHovered}
          onOpen={n => {
            if (n.kind === 'summary') void openSummary(n);
          }}
          onError={() => setPixiFailed(true)}
          onReady={fireReady}
        />
      ) : (
        <svg
          ref={svgRef}
          viewBox={`0 0 ${VIEWPORT_W} ${VIEWPORT_H}`}
          className="block w-full touch-none select-none"
          style={{
            height: 'min(640px, calc(100vh - 22rem))',
            cursor: grabbing ? 'grabbing' : 'grab',
          }}
          onPointerDown={onBackgroundPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={endDrag}
          onPointerLeave={endDrag}
          onWheel={onWheelZoom}
          data-testid="memory-graph-svg">
          {/* Pan / zoom group — drag the background to pan, scroll to zoom. */}
          <g transform={`translate(${view.tx} ${view.ty}) scale(${view.scale})`}>
            <g
              stroke={isDark ? '#cbd5e1' : '#475569'}
              strokeWidth={isDark ? 0.6 : 1.2}
              opacity={isDark ? 0.7 : 0.7}>
              {sim.edges.map(([ai, bi], idx) => {
                // Only draw edges whose endpoints are both mounted yet.
                if (ai >= svgVisible || bi >= svgVisible) return null;
                const a = sim.sim[ai];
                const b = sim.sim[bi];
                return (
                  <line
                    key={idx}
                    ref={el => {
                      lineEls.current[idx] = el;
                    }}
                    x1={a.x}
                    y1={a.y}
                    x2={b.x}
                    y2={b.y}
                  />
                );
              })}
            </g>
            <g>
              {sim.sim.slice(0, svgVisible).map((n, i) => {
                const r = nodeRadius(n);
                const fill = nodeColor(n);
                const isHover = hovered?.id === n.id;
                // Leaves stay flat; summary / contact nodes glow in their
                // own colour so the tree levels "light up".
                const glow =
                  n.kind === 'chunk' ? undefined : `drop-shadow(0 0 ${isHover ? 7 : 4}px ${fill})`;
                return (
                  <circle
                    key={n.id}
                    ref={el => {
                      circleEls.current[i] = el;
                    }}
                    cx={n.x}
                    cy={n.y}
                    r={isHover ? r + 2 : r}
                    fill={fill}
                    stroke={
                      isHover ? (isDark ? '#0f172a' : '#1e293b') : isDark ? '#ffffff' : '#e2e8f0'
                    }
                    strokeWidth={isHover ? 1.4 : 0.8}
                    style={{ cursor: grabbing ? 'grabbing' : 'pointer', filter: glow }}
                    onPointerDown={e => onNodePointerDown(e, n)}
                    onMouseEnter={() => setHovered(n)}
                    onClick={() => {
                      // A drag ends with a click event too — skip the open
                      // when the pointer actually moved.
                      if (movedRef.current) return;
                      if (n.kind === 'summary') void openSummary(n);
                    }}
                    data-testid={`memory-graph-node-${n.id}`}>
                    <title>{tooltipFor(n, t)}</title>
                  </circle>
                );
              })}
            </g>
          </g>
        </svg>
      )}
      {hovered && (
        <div
          className="border-t border-line-subtle bg-surface-muted/70 dark:bg-surface/70 px-4 py-2 text-xs text-content-secondary"
          data-testid="memory-graph-tooltip">
          {hovered.kind === 'root' ? (
            <span className="font-medium text-violet-600 dark:text-violet-400">
              {hovered.label}
            </span>
          ) : hovered.kind === 'source' ? (
            <span className="font-medium text-orange-600 dark:text-orange-400">
              {hovered.label}
            </span>
          ) : hovered.kind === 'summary' ? (
            <>
              <span className="font-mono">L{hovered.level ?? '?'}</span>
              <span className="text-content-faint"> · </span>
              <span className="capitalize">{hovered.tree_kind}</span>
              <span className="text-content-faint"> · </span>
              <span>{hovered.tree_scope}</span>
              <span className="text-content-faint"> · </span>
              <span>
                {hovered.child_count ?? 0} {t('graph.children')}
              </span>
              {hoveredSummaryPath && (
                <>
                  <span className="ml-3 break-all font-mono text-content-faint">
                    workspace:{hoveredSummaryPath}
                  </span>
                  <Button
                    variant="secondary"
                    size="xs"
                    data-testid={`memory-graph-preview-${hovered.id}`}
                    disabled={previewingPath === hoveredSummaryPath}
                    onClick={() => void previewSummary(hovered)}
                    className="ml-3 text-[11px] shadow-sm">
                    {previewingPath === hoveredSummaryPath
                      ? t('migration.previewRunning')
                      : t('migration.previewAction')}
                  </Button>
                </>
              )}
            </>
          ) : hovered.kind === 'contact' ? (
            <>
              <span className="font-medium text-violet-700 dark:text-violet-300">
                {hovered.label}
              </span>
              <span className="ml-3 text-content-faint">
                {t('graph.person')} · canonical id {hovered.id.slice(0, 12)}…
              </span>
            </>
          ) : (
            <>
              <span className="font-medium">{hovered.label || 'chunk'}</span>
              <span className="ml-3 text-content-faint">{t('graph.document')}</span>
            </>
          )}
        </div>
      )}
      {preview && (
        <div
          className="border-t border-line-subtle bg-surface px-4 py-3 dark:bg-surface-canvas"
          data-testid="memory-graph-preview">
          <div className="mb-2 break-all font-mono text-[11px] text-content-faint">
            workspace:{preview.path}
          </div>
          <pre className="max-h-40 overflow-auto whitespace-pre-wrap rounded-md bg-surface-muted p-3 text-xs text-content-secondary">
            {preview.error || preview.contents}
            {preview.truncated ? '\n…' : ''}
          </pre>
        </div>
      )}
    </div>
  );
}

function tooltipFor(n: GraphNode, t: (key: string, fallback?: string) => string): string {
  if (n.kind === 'root') return n.label;
  if (n.kind === 'summary') return t('graph.tooltip.summary');
  if (n.kind === 'contact') return t('graph.tooltip.contact');
  return n.label || t('graph.document');
}
