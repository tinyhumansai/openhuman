/**
 * Position seeding for the SVG force layout, with incremental carry-over.
 *
 * When the graph data changes (a background sync completes, a rebuild, a mode
 * toggle) the component re-derives its node array. Re-seeding every node from
 * scratch makes the whole graph reshuffle and re-settle — jarring on a live
 * update. Instead this:
 *   - keeps each surviving node (same `id`) at its previous position,
 *   - seeds a genuinely-new node near its parent's previous position (tree
 *     mode) so it eases in next to where it belongs, falling back to a
 *     deterministic ring around the viewport centre,
 *   - reports a `reheatAlpha`: a gentle 0.3 when anything carried over (so the
 *     sim nudges rather than explodes), or a full 1 for a first / fully-new
 *     graph.
 *
 * Pure and deterministic (no RNG / clock) so it is straightforward to test.
 */
import { type GraphEdge, type GraphMode, type GraphNode } from '../../utils/tauriCommands';
import { VIEWPORT_H, VIEWPORT_W } from './memoryGraphLayout';

export interface SeededPosition {
  x: number;
  y: number;
  /** True when the node had no carried-over position (newly arrived). */
  isNew: boolean;
}

export interface SeedResult {
  /** Index-aligned with the input `nodes`. */
  positions: SeededPosition[];
  /** Edge index pairs [childIdx, parentIdx] (tree) or [fromIdx, toIdx] (contacts). */
  edges: Array<[number, number]>;
  /** How many nodes had no previous position. */
  newCount: number;
  /** Initial simulation alpha: gentle (0.3) on an incremental update, full (1) otherwise. */
  reheatAlpha: number;
}

const CX = VIEWPORT_W / 2;
const CY = VIEWPORT_H / 2;
const NEW_NODE_OFFSET = 24;

/** Deterministic ring position around the viewport centre for index `i`. */
function ringPosition(i: number, total: number): { x: number; y: number } {
  const angle = (i / Math.max(1, total)) * Math.PI * 2;
  const r = 200 + (i % 7) * 12;
  return { x: CX + Math.cos(angle) * r, y: CY + Math.sin(angle) * r };
}

export function seedSvgLayout(
  nodes: GraphNode[],
  edges: GraphEdge[],
  mode: GraphMode,
  prev: ReadonlyMap<string, { x: number; y: number }>
): SeedResult {
  const idIndex = new Map<string, number>();
  nodes.forEach((n, i) => idIndex.set(n.id, i));

  const positions: SeededPosition[] = nodes.map((n, i) => {
    const carried = prev.get(n.id);
    if (carried) return { x: carried.x, y: carried.y, isNew: false };

    // New node — seed near its parent's previous position if we know it.
    if (mode === 'tree' && n.parent_id) {
      const parent = prev.get(n.parent_id);
      if (parent) {
        const a = (i % 12) * (Math.PI / 6);
        return {
          x: parent.x + Math.cos(a) * NEW_NODE_OFFSET,
          y: parent.y + Math.sin(a) * NEW_NODE_OFFSET,
          isNew: true,
        };
      }
    }
    const ring = ringPosition(i, nodes.length);
    return { x: ring.x, y: ring.y, isNew: true };
  });

  const edgeIndices: Array<[number, number]> = [];
  if (mode === 'tree') {
    for (const n of nodes) {
      if (!n.parent_id) continue;
      const child = idIndex.get(n.id);
      const parent = idIndex.get(n.parent_id);
      if (child == null || parent == null) continue;
      edgeIndices.push([child, parent]);
    }
  } else {
    for (const e of edges) {
      const a = idIndex.get(e.from);
      const b = idIndex.get(e.to);
      if (a == null || b == null) continue;
      edgeIndices.push([a, b]);
    }
  }

  const newCount = positions.reduce((acc, p) => acc + (p.isNew ? 1 : 0), 0);
  // Gentle reheat only when at least one node survived (an incremental update);
  // a first load or a fully-replaced graph gets a full settle.
  const reheatAlpha = nodes.length > 0 && newCount < nodes.length ? 0.3 : 1;

  return { positions, edges: edgeIndices, newCount, reheatAlpha };
}
