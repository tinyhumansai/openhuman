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
  useMemo,
  useReducer,
  useRef,
  useState,
} from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { type GraphEdge, type GraphMode, type GraphNode } from '../../utils/tauriCommands';
import { openWorkspacePath, previewWorkspaceText } from '../../utils/tauriCommands/workspacePaths';
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

interface MemoryGraphProps {
  /** Pre-fetched summary / chunk / contact nodes. */
  nodes: GraphNode[];
  /** Explicit edges (only used in contacts mode). */
  edges: GraphEdge[];
  /** Which graph this is — drives colour palette + click behaviour. */
  mode: GraphMode;
  /** Optional override for the empty-state message. */
  emptyHint?: string;
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

export function MemoryGraph({ nodes, edges, mode, emptyHint }: MemoryGraphProps) {
  const { t } = useT();
  const [hovered, setHovered] = useState<GraphNode | null>(null);
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
    // SVG fallback resets its transform; the Pixi canvas listens on the
    // reset signal. Both are bumped so the button works in either path.
    setView({ tx: 0, ty: 0, scale: 1 });
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

  // Build edges and (for the SVG fallback) seed positions + relax. The
  // O(n²) relax only runs when WebGL is unavailable; the Pixi path runs
  // its own d3-force simulation instead.
  const sim = useMemo(() => {
    if (!nodes || nodes.length === 0) return null;
    const idIndex = new Map<string, number>();
    nodes.forEach((n, i) => idIndex.set(n.id, i));
    const sim: SimNode[] = nodes.map((n, i) => {
      const angle = (i / nodes.length) * Math.PI * 2;
      const r = 200 + (i % 7) * 12;
      return {
        ...n,
        x: VIEWPORT_W / 2 + Math.cos(angle) * r,
        y: VIEWPORT_H / 2 + Math.sin(angle) * r,
        vx: 0,
        vy: 0,
      };
    });
    const edgeIndices: Array<[number, number]> = [];
    if (mode === 'tree') {
      // Tree mode: each summary's parent_id is the edge.
      for (const n of nodes) {
        if (!n.parent_id) continue;
        const childIdx = idIndex.get(n.id);
        const parentIdx = idIndex.get(n.parent_id);
        if (childIdx == null || parentIdx == null) continue;
        edgeIndices.push([childIdx, parentIdx]);
      }
    } else {
      for (const e of edges) {
        const a = idIndex.get(e.from);
        const b = idIndex.get(e.to);
        if (a == null || b == null) continue;
        edgeIndices.push([a, b]);
      }
    }
    if (!useWebGL) relaxLayout(sim, edgeIndices);
    return { sim, edges: edgeIndices };
  }, [nodes, edges, mode, useWebGL]);

  if (nodes.length === 0) {
    return (
      <div
        className="flex h-[640px] items-center justify-center rounded-lg border border-stone-100 dark:border-neutral-800 bg-stone-50/40 text-sm text-stone-500 dark:text-neutral-400"
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
      className="memory-graph rounded-lg border border-stone-100 dark:border-neutral-800 bg-white dark:bg-neutral-900"
      onMouseLeave={() => setHovered(null)}>
      <div className="flex items-center justify-between gap-4 border-b border-stone-100 dark:border-neutral-800 px-4 py-2">
        <div className="flex items-center gap-3 text-xs text-stone-500 dark:text-neutral-400">
          <span>
            {nodes.length} {t('graph.nodes')}
          </span>
          <span className="text-stone-300 dark:text-neutral-600">·</span>
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
              className="flex items-center gap-1.5 text-xs text-stone-600 dark:text-neutral-300">
              <span
                className="inline-block h-2.5 w-2.5 rounded-full"
                style={{ backgroundColor: item.color }}
              />
              {item.label}
            </span>
          ))}
          <button
            type="button"
            onClick={resetView}
            data-testid="memory-graph-reset-view"
            className="rounded-md border border-stone-200 bg-white px-2 py-1 text-[11px] font-medium text-stone-600 shadow-sm hover:bg-stone-50 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-300 dark:hover:bg-neutral-800">
            {t('graph.resetView')}
          </button>
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
            <g stroke="#cbd5e1" strokeWidth={0.6} opacity={0.7}>
              {sim.edges.map(([ai, bi], idx) => {
                const a = sim.sim[ai];
                const b = sim.sim[bi];
                return <line key={idx} x1={a.x} y1={a.y} x2={b.x} y2={b.y} />;
              })}
            </g>
            <g>
              {sim.sim.map(n => {
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
                    cx={n.x}
                    cy={n.y}
                    r={isHover ? r + 2 : r}
                    fill={fill}
                    stroke={isHover ? '#0f172a' : '#ffffff'}
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
          className="border-t border-stone-100 dark:border-neutral-800 bg-stone-50/70 dark:bg-neutral-900/70 px-4 py-2 text-xs text-stone-700 dark:text-neutral-200"
          data-testid="memory-graph-tooltip">
          {hovered.kind === 'source' ? (
            <span className="font-medium text-orange-600 dark:text-orange-400">
              {hovered.label}
            </span>
          ) : hovered.kind === 'summary' ? (
            <>
              <span className="font-mono">L{hovered.level ?? '?'}</span>
              <span className="text-stone-400 dark:text-neutral-500"> · </span>
              <span className="capitalize">{hovered.tree_kind}</span>
              <span className="text-stone-400 dark:text-neutral-500"> · </span>
              <span>{hovered.tree_scope}</span>
              <span className="text-stone-400 dark:text-neutral-500"> · </span>
              <span>
                {hovered.child_count ?? 0} {t('graph.children')}
              </span>
              {hoveredSummaryPath && (
                <>
                  <span className="ml-3 break-all font-mono text-stone-400 dark:text-neutral-500">
                    workspace:{hoveredSummaryPath}
                  </span>
                  <button
                    type="button"
                    data-testid={`memory-graph-preview-${hovered.id}`}
                    disabled={previewingPath === hoveredSummaryPath}
                    onClick={() => void previewSummary(hovered)}
                    className="ml-3 rounded-md border border-stone-200 bg-white px-2 py-1 text-[11px] font-medium text-stone-700 shadow-sm hover:bg-stone-50 disabled:cursor-not-allowed disabled:opacity-60 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-200 dark:hover:bg-neutral-800">
                    {previewingPath === hoveredSummaryPath
                      ? t('migration.previewRunning')
                      : t('migration.previewAction')}
                  </button>
                </>
              )}
            </>
          ) : hovered.kind === 'contact' ? (
            <>
              <span className="font-medium text-violet-700 dark:text-violet-300">
                {hovered.label}
              </span>
              <span className="ml-3 text-stone-400 dark:text-neutral-500">
                {t('graph.person')} · canonical id {hovered.id.slice(0, 12)}…
              </span>
            </>
          ) : (
            <>
              <span className="font-medium">{hovered.label || 'chunk'}</span>
              <span className="ml-3 text-stone-400 dark:text-neutral-500">
                {t('graph.document')}
              </span>
            </>
          )}
        </div>
      )}
      {preview && (
        <div
          className="border-t border-stone-100 bg-white px-4 py-3 dark:border-neutral-800 dark:bg-neutral-950"
          data-testid="memory-graph-preview">
          <div className="mb-2 break-all font-mono text-[11px] text-stone-400 dark:text-neutral-500">
            workspace:{preview.path}
          </div>
          <pre className="max-h-40 overflow-auto whitespace-pre-wrap rounded-md bg-stone-50 p-3 text-xs text-stone-700 dark:bg-neutral-900 dark:text-neutral-200">
            {preview.error || preview.contents}
            {preview.truncated ? '\n…' : ''}
          </pre>
        </div>
      )}
    </div>
  );
}

function tooltipFor(n: GraphNode, t: (key: string, fallback?: string) => string): string {
  // NOTE: the underlying t() does not interpolate params; placeholders in the
  // translated string are rendered as-is. Preserved to match prior behavior.
  if (n.kind === 'summary') return t('graph.tooltip.summary');
  if (n.kind === 'contact') return t('graph.tooltip.contact');
  return n.label || t('graph.document');
}
