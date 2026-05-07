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
 * Click a node → opens the matching `.md` file in Obsidian via the
 * `obsidian://open?path=...` deep link, dispatched through Tauri's
 * `plugin-opener` so the OS shell handles the URL scheme. Without that
 * shim the webview tries to navigate itself and Obsidian never opens.
 *
 * Pure SVG, no external graph dep — keeps the bundle small and the
 * rendering deterministic for tests/screenshots.
 */
import { useMemo, useRef, useState } from 'react';

import { openUrl } from '../../utils/openUrl';
import { type GraphEdge, type GraphMode, type GraphNode } from '../../utils/tauriCommands';

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
  /** Absolute path to the content root, also from the RPC. */
  contentRootAbs: string;
  /** Optional override for the empty-state message. */
  emptyHint?: string;
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

/**
 * Open a summary's `.md` file in Obsidian via the OS shell. Custom URL
 * schemes go through `tauri-plugin-opener` so the host app handles
 * them — `window.location.href` would route through the embedded
 * webview's intent handler and either no-op or navigate the
 * MemoryWorkspace away.
 */
async function openSummaryInObsidian(node: GraphNode, contentRootAbs: string): Promise<void> {
  if (node.kind !== 'summary' || !node.tree_kind || node.level == null || !node.file_basename) {
    return;
  }
  const slug = slugify(node.tree_scope ?? '');
  // Mirrors `summary_rel_path` on the Rust side — the `wiki/` prefix
  // separates derived/processed content from the raw upstream archive
  // under `raw/`. Folder name is `<kind>-<scope>` (flattened from the
  // legacy two-level layout) so the Obsidian sidebar listing stays
  // readable.
  const rel =
    node.tree_kind === 'global'
      ? `wiki/summaries`
      : `wiki/summaries/${node.tree_kind}-${slug}/L${node.level}/${node.file_basename}.md`;
  const abs = joinPath(contentRootAbs, rel);
  const url = `obsidian://open?path=${encodeURIComponent(abs)}`;
  console.debug('[memory-graph] open in Obsidian url=%s', url);
  try {
    await openUrl(url);
  } catch (err) {
    console.error('[memory-graph] openUrl failed', err);
  }
}

/** Mirror of `paths::slugify_source_id` (Rust). */
function slugify(s: string): string {
  const lower = s.toLowerCase();
  let out = '';
  let lastDash = true;
  let pendingUnderscore = false;
  for (const ch of lower) {
    if (ch === '_') {
      if (!lastDash) pendingUnderscore = true;
    } else if (/[a-z0-9]/.test(ch)) {
      if (pendingUnderscore) {
        out += '_';
        pendingUnderscore = false;
      }
      out += ch;
      lastDash = false;
    } else {
      pendingUnderscore = false;
      if (!lastDash) {
        out += '-';
        lastDash = true;
      }
    }
  }
  return out.replace(/[-_]+$/, '').slice(0, 120) || 'unknown';
}

/** Cross-platform path join (forward slash; Obsidian accepts both). */
function joinPath(root: string, rel: string): string {
  const trimmed = root.endsWith('/') || root.endsWith('\\') ? root.slice(0, -1) : root;
  return `${trimmed}/${rel}`;
}

export function MemoryGraph({ nodes, edges, mode, contentRootAbs, emptyHint }: MemoryGraphProps) {
  const [hovered, setHovered] = useState<GraphNode | null>(null);
  const svgRef = useRef<SVGSVGElement | null>(null);

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
        className="flex h-[640px] items-center justify-center rounded-lg border border-stone-100 bg-stone-50/40 text-sm text-stone-500"
        data-testid="memory-graph-empty">
        {emptyHint ??
          (mode === 'contacts'
            ? 'No contact mentions yet — sync a source and rebuild trees first.'
            : 'No memory yet — connect a source above to start ingesting.')}
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
            label: kind === 'source' ? 'Source' : kind === 'topic' ? 'Topic' : 'Global',
            color: SUMMARY_TREE_COLOR[kind] ?? '#94a3b8',
          }))
      : [
          { label: 'Document', color: NODE_COLOR.chunk },
          { label: 'Contact', color: NODE_COLOR.contact },
        ];

  return (
    <div className="memory-graph rounded-lg border border-stone-100 bg-white">
      <div className="flex items-center justify-between gap-4 border-b border-stone-100 px-4 py-2">
        <div className="flex items-center gap-3 text-xs text-stone-500">
          <span>{nodes.length} nodes</span>
          <span className="text-stone-300">·</span>
          <span>
            {sim.edges.length} {mode === 'tree' ? 'parent → child' : 'document ↔ contact'} link
            {sim.edges.length === 1 ? '' : 's'}
          </span>
        </div>
        <div className="flex items-center gap-3">
          {legend.map(item => (
            <span key={item.label} className="flex items-center gap-1.5 text-xs text-stone-600">
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
                onMouseLeave={() => setHovered(prev => (prev?.id === n.id ? null : prev))}
                onClick={() => {
                  if (n.kind === 'summary') void openSummaryInObsidian(n, contentRootAbs);
                }}
                data-testid={`memory-graph-node-${n.id}`}>
                <title>{tooltipFor(n)}</title>
              </circle>
            );
          })}
        </g>
      </svg>
      {hovered && (
        <div
          className="border-t border-stone-100 bg-stone-50/70 px-4 py-2 text-xs text-stone-700"
          data-testid="memory-graph-tooltip">
          {hovered.kind === 'summary' ? (
            <>
              <span className="font-mono">L{hovered.level ?? '?'}</span>
              <span className="text-stone-400"> · </span>
              <span className="capitalize">{hovered.tree_kind}</span>
              <span className="text-stone-400"> · </span>
              <span>{hovered.tree_scope}</span>
              <span className="text-stone-400"> · </span>
              <span>{hovered.child_count ?? 0} children</span>
              <span className="ml-3 text-stone-400">click to open in Obsidian</span>
            </>
          ) : hovered.kind === 'contact' ? (
            <>
              <span className="font-medium text-violet-700">{hovered.label}</span>
              <span className="ml-3 text-stone-400">
                person · canonical id {hovered.id.slice(0, 12)}…
              </span>
            </>
          ) : (
            <>
              <span className="font-medium">{hovered.label || 'chunk'}</span>
              <span className="ml-3 text-stone-400">document</span>
            </>
          )}
        </div>
      )}
    </div>
  );
}

function tooltipFor(n: GraphNode): string {
  if (n.kind === 'summary') {
    return `L${n.level ?? 0} · ${n.tree_kind} · ${n.tree_scope ?? ''} · ${n.child_count ?? 0} children`;
  }
  if (n.kind === 'contact') return `${n.label} (person)`;
  return n.label || 'document';
}
