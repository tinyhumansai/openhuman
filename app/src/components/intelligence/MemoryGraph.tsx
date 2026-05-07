/**
 * Obsidian-style force-directed graph view for the memory tree.
 *
 * Reads every sealed summary via `memory_tree_graph_export` and lays them
 * out using a tiny barycentric force simulation:
 *   - parent → child links pull connected nodes together
 *   - all-pairs Coulomb repulsion pushes overlapping nodes apart
 *   - centring force keeps the cloud anchored in the viewport
 *
 * Trees naturally cluster by `tree_id` because edges only exist within a
 * tree — no cross-tree spaghetti. Node colour encodes `tree_kind`
 * (source / topic / global), node radius encodes `level` so roots read
 * larger than leaves at a glance.
 *
 * Click a node → opens the matching `.md` file in Obsidian via the
 * `obsidian://open?path=...` deep link. The absolute path comes from the
 * RPC response so it works regardless of where the workspace lives.
 *
 * Pure SVG, no external graph dep — keeps the bundle small and the
 * rendering deterministic for tests/screenshots.
 */
import { useMemo, useRef, useState } from 'react';

import { type GraphNode } from '../../utils/tauriCommands';

interface SimNode extends GraphNode {
  x: number;
  y: number;
  vx: number;
  vy: number;
}

interface MemoryGraphProps {
  /** Pre-fetched summary nodes from `memory_tree_graph_export`. */
  nodes: GraphNode[];
  /** Absolute path to the content root, also from the RPC. */
  contentRootAbs: string;
  /** Optional override for the empty-state message. */
  emptyHint?: string;
}

const KIND_COLOR: Record<string, string> = {
  source: '#4A83DD', // ocean
  topic: '#E8A653', // amber
  global: '#7BB489', // sage
};

const KIND_LABEL: Record<string, string> = {
  source: 'Source',
  topic: 'Topic',
  global: 'Global',
};

const VIEWPORT_W = 1100;
const VIEWPORT_H = 640;

/** Radius in px for a node at the given level (roots are largest). */
function nodeRadius(level: number): number {
  return Math.max(4, 10 - level * 0.8);
}

/**
 * Run the force simulation for `iterations` ticks. Mutates positions in
 * place so we can re-use the same buffer across renders.
 */
function relaxLayout(
  nodes: SimNode[],
  edges: Array<[number, number]>,
  iterations = 220
): void {
  const REPULSION = 1800;
  const SPRING_K = 0.04;
  const SPRING_LEN = 60;
  const CENTER_K = 0.0025;
  const FRICTION = 0.85;
  const cx = VIEWPORT_W / 2;
  const cy = VIEWPORT_H / 2;

  for (let iter = 0; iter < iterations; iter++) {
    // Coulomb repulsion (all pairs — fine for a few thousand nodes).
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
    // Spring attraction along edges.
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
    // Centring + friction + integration.
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
 * Open the matching `.md` file in Obsidian. We always have the absolute
 * `content_root` and the file's relative path inside it, so we use
 * `obsidian://open?path=<absolute path>` which Obsidian handles without
 * needing the vault to be pre-registered.
 *
 * The relative path matches `summary_rel_path` on the Rust side:
 *   summaries/<tree_kind>/<scope_slug>/L<level>/<basename>.md
 *
 * `tree_scope` already arrives slugified at the SQL level for source/topic
 * trees; for `global` it's the date label like `"global"` and the scope
 * directory is the date — but for the deep-link demo we accept that and
 * fall through to the content root if the rel-path can't be derived.
 */
function openInObsidian(node: GraphNode, contentRootAbs: string): void {
  // We only have the level and basename from the RPC; the scope slug for
  // source/topic comes from the tree's `tree_scope` (already slugified by
  // the slug rules in `paths::slugify_source_id`). For `global` we don't
  // have the per-day directory in this RPC, so we fall back to opening
  // the content root and let the user navigate.
  const slug = slugify(node.tree_scope);
  const rel =
    node.tree_kind === 'global'
      ? `summaries/global` // open the directory; Obsidian will list dailies
      : `summaries/${node.tree_kind}/${slug}/L${node.level}/${node.file_basename}.md`;
  const abs = joinPath(contentRootAbs, rel);
  const url = `obsidian://open?path=${encodeURIComponent(abs)}`;
  console.debug('[memory-graph] open in Obsidian url=%s', url);
  window.location.href = url;
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

export function MemoryGraph({ nodes, contentRootAbs, emptyHint }: MemoryGraphProps) {
  const [hovered, setHovered] = useState<GraphNode | null>(null);
  const svgRef = useRef<SVGSVGElement | null>(null);

  // Run the force simulation once when nodes arrive. Memoised so panning /
  // zooming the SVG doesn't re-run physics.
  const sim = useMemo(() => {
    if (!nodes || nodes.length === 0) return null;
    const idIndex = new Map<string, number>();
    nodes.forEach((n, i) => idIndex.set(n.id, i));
    // Seed positions in a circle so the simulation has somewhere to start.
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
    const edges: Array<[number, number]> = [];
    for (const n of nodes) {
      if (!n.parent_id) continue;
      const childIdx = idIndex.get(n.id);
      const parentIdx = idIndex.get(n.parent_id);
      if (childIdx == null || parentIdx == null) continue;
      edges.push([childIdx, parentIdx]);
    }
    relaxLayout(sim, edges);
    return { sim, edges };
  }, [nodes]);

  if (nodes.length === 0) {
    return (
      <div
        className="flex h-[640px] items-center justify-center rounded-lg border border-stone-100 bg-stone-50/40 text-sm text-stone-500"
        data-testid="memory-graph-empty">
        {emptyHint ?? 'No memory yet — connect a source above to start ingesting.'}
      </div>
    );
  }

  if (!sim) return null;

  // Distinct kinds for the legend — preserves first-seen order.
  const kindsInUse = Array.from(new Set(nodes.map(n => n.tree_kind)));

  return (
    <div className="memory-graph rounded-lg border border-stone-100 bg-white">
      <div className="flex items-center justify-between gap-4 border-b border-stone-100 px-4 py-2">
        <div className="flex items-center gap-3 text-xs text-stone-500">
          <span>{nodes.length} summary nodes</span>
          <span className="text-stone-300">·</span>
          <span>{sim.edges.length} parent → child links</span>
        </div>
        <div className="flex items-center gap-3">
          {kindsInUse.map(kind => (
            <span key={kind} className="flex items-center gap-1.5 text-xs text-stone-600">
              <span
                className="inline-block h-2.5 w-2.5 rounded-full"
                style={{ backgroundColor: KIND_COLOR[kind] ?? '#94a3b8' }}
              />
              {KIND_LABEL[kind] ?? kind}
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
        {/* Edges first so they paint behind nodes. */}
        <g stroke="#cbd5e1" strokeWidth={0.6} opacity={0.7}>
          {sim.edges.map(([ai, bi], idx) => {
            const a = sim.sim[ai];
            const b = sim.sim[bi];
            return <line key={idx} x1={a.x} y1={a.y} x2={b.x} y2={b.y} />;
          })}
        </g>
        <g>
          {sim.sim.map(n => {
            const r = nodeRadius(n.level);
            const fill = KIND_COLOR[n.tree_kind] ?? '#94a3b8';
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
                onClick={() => openInObsidian(n, contentRootAbs)}
                data-testid={`memory-graph-node-${n.id}`}>
                <title>{`L${n.level} · ${n.tree_kind} · ${n.tree_scope} · ${n.child_count} children`}</title>
              </circle>
            );
          })}
        </g>
      </svg>
      {hovered && (
        <div
          className="border-t border-stone-100 bg-stone-50/70 px-4 py-2 text-xs text-stone-700"
          data-testid="memory-graph-tooltip">
          <span className="font-mono">L{hovered.level}</span>
          <span className="text-stone-400"> · </span>
          <span className="capitalize">{hovered.tree_kind}</span>
          <span className="text-stone-400"> · </span>
          <span>{hovered.tree_scope}</span>
          <span className="text-stone-400"> · </span>
          <span>{hovered.child_count} children</span>
          <span className="ml-3 text-stone-400">click to open in Obsidian</span>
        </div>
      )}
    </div>
  );
}
