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
 * Click a summary node → opens the matching `.md` file through the
 * shared workspace path command. This keeps Memory graph file actions on
 * the same guarded contract as chat workspace links.
 *
 * Pure SVG, no external graph dep — keeps the bundle small and the
 * rendering deterministic for tests/screenshots.
 */
import { useCallback, useMemo, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { type GraphEdge, type GraphMode, type GraphNode } from '../../utils/tauriCommands';
import { openWorkspacePath, previewWorkspaceText } from '../../utils/tauriCommands/workspacePaths';
import { summaryWorkspacePath } from './memoryWorkspacePaths';

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

/** Per-node-kind palette. Source/topic/global preserved for tree mode. */
const SUMMARY_TREE_COLOR: Record<string, string> = {
  source: '#4A83DD',
  topic: '#E8A653',
  global: '#7BB489',
};
const NODE_COLOR: Record<string, string> = {
  chunk: '#4A83DD',
  contact: '#A78BFA', // violet — matches the Obsidian button accent
};

const VIEWPORT_W = 1100;
const VIEWPORT_H = 640;

function nodeColor(node: GraphNode): string {
  if (node.kind === 'summary') {
    return SUMMARY_TREE_COLOR[node.tree_kind ?? ''] ?? '#94a3b8';
  }
  return NODE_COLOR[node.kind] ?? '#94a3b8';
}

function nodeRadius(node: GraphNode): number {
  if (node.kind === 'summary') {
    return Math.max(4, 10 - (node.level ?? 0) * 0.8);
  }
  if (node.kind === 'contact') return 9;
  return 4; // chunk
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

  // Run the force simulation once when nodes arrive. Memoised so panning /
  // zooming the SVG doesn't re-run physics.
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
    relaxLayout(sim, edgeIndices);
    return { sim, edges: edgeIndices };
  }, [nodes, edges, mode]);

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

  // Distinct legend rows for the active mode.
  const legend =
    mode === 'tree'
      ? Array.from(new Set(nodes.map(n => n.tree_kind ?? '')))
          .filter(Boolean)
          .map(kind => ({
            label:
              kind === 'source'
                ? t('graph.source')
                : kind === 'topic'
                  ? t('graph.topic')
                  : t('graph.global'),
            color: SUMMARY_TREE_COLOR[kind] ?? '#94a3b8',
          }))
      : [
          { label: t('graph.document'), color: NODE_COLOR.chunk },
          { label: t('graph.contact'), color: NODE_COLOR.contact },
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
        </div>
      </div>
      <svg
        ref={svgRef}
        viewBox={`0 0 ${VIEWPORT_W} ${VIEWPORT_H}`}
        className="block w-full"
        style={{ height: 'min(640px, calc(100vh - 22rem))', cursor: 'grab' }}
        data-testid="memory-graph-svg">
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
            return (
              <circle
                key={n.id}
                cx={n.x}
                cy={n.y}
                r={isHover ? r + 2 : r}
                fill={fill}
                stroke={isHover ? '#0f172a' : '#ffffff'}
                strokeWidth={isHover ? 1.4 : 0.8}
                style={{ cursor: 'pointer', transition: 'r 120ms ease' }}
                onMouseEnter={() => setHovered(n)}
                onClick={() => {
                  if (n.kind === 'summary') void openSummary(n);
                }}
                data-testid={`memory-graph-node-${n.id}`}>
                <title>{tooltipFor(n, t)}</title>
              </circle>
            );
          })}
        </g>
      </svg>
      {hovered && (
        <div
          className="border-t border-stone-100 dark:border-neutral-800 bg-stone-50/70 px-4 py-2 text-xs text-stone-700 dark:text-neutral-200"
          data-testid="memory-graph-tooltip">
          {hovered.kind === 'summary' ? (
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
