/**
 * Shared, render-agnostic layout + palette helpers for the memory graph.
 *
 * Physics is d3-force (Barnes–Hut quadtree charge, O(n log n)) so the
 * 1000-node cap settles smoothly — the same model Obsidian's graph is
 * built on. Both the WebGL (Pixi) renderer and the SVG fallback consume
 * these helpers so colours, radii, edge derivation and hit-testing stay
 * identical across paths.
 */
import {
  forceCenter,
  forceCollide,
  forceLink,
  forceManyBody,
  forceSimulation,
  type Simulation,
  type SimulationLinkDatum,
  type SimulationNodeDatum,
} from 'd3-force';

import { type GraphEdge, type GraphMode, type GraphNode } from '../../utils/tauriCommands';

/**
 * Per-level palette — each tree level "lights up" in its own hue
 * (mirrors the Obsidian `path:L{n}` colour groups).
 */
export const LEVEL_COLOR = [
  '#7C3AED', // L0
  '#4A83DD', // L1
  '#1FB6C7', // L2
  '#34C77B', // L3
  '#E8A653', // L4
  '#E0654A', // L5
  '#C026D3', // L6+
];
export const LEAF_COLOR = '#94A3B8'; // raw chunks / leaves (no level)
export const CONTACT_COLOR = '#A78BFA'; // person entities (contacts mode)
export const SOURCE_COLOR = '#F97316'; // synthetic source root nodes
export const ROOT_COLOR = '#8B5CF6'; // master root hub (purple)

/** Layout is computed in this fixed coordinate space; the renderer pans/zooms it. */
export const VIEWPORT_W = 1100;
export const VIEWPORT_H = 640;
// Lower bound shared by auto-fit framing and manual wheel zoom-out. Kept very
// small (20× zoom-out) so large clouds — e.g. a Notion connection's hundreds
// of page-chunk leaves — can be framed in full. At 0.3 the auto-fit was
// clamped above the scale needed to show every node, so big graphs rendered
// "too zoomed in" with the outer nodes spilling off-screen. Using one shared
// floor (rather than a separate, lower auto-fit floor) avoids a zoom-snap
// where the first wheel tick would jump back up to the manual floor.
export const ZOOM_MIN = 0.05;
export const ZOOM_MAX = 4;

export function levelColor(level: number | null | undefined): string {
  if (level == null) return LEAF_COLOR;
  return LEVEL_COLOR[Math.max(0, level) % LEVEL_COLOR.length];
}

export function nodeColor(node: GraphNode): string {
  if (node.kind === 'root') return ROOT_COLOR;
  if (node.kind === 'source') return SOURCE_COLOR;
  if (node.kind === 'summary') return levelColor(node.level);
  if (node.kind === 'contact') return CONTACT_COLOR;
  return LEAF_COLOR; // chunk
}

export function nodeRadius(node: GraphNode): number {
  if (node.kind === 'root') return 20;
  if (node.kind === 'source') return 16;
  if (node.kind === 'summary') {
    // Higher levels render slightly larger, but the size MUST be capped:
    // document source trees place their cross-document merge tier at a large
    // synthetic level (MERGE_LEVEL_BASE = 1000+), so the raw `level * 2.5`
    // would explode to thousands of px — rendering giant discs and, via the
    // `forceCollide(nodeRadius + 2)` term, blowing the whole layout apart.
    // The cap keeps merge nodes the largest summaries without distorting it.
    const level = node.level ?? 0;
    return Math.min(5 + level * 2.5, 14);
  }
  if (node.kind === 'contact') return 9;
  return 3; // chunk / document leaf
}

/** Source / summary / contact nodes glow; leaves stay flat so the structure pops. */
export function nodeGlows(node: GraphNode): boolean {
  return node.kind !== 'chunk';
}

/** A graph node carrying mutable physics state (x/y/vx/vy populated by d3-force). */
export interface SimNode extends GraphNode, SimulationNodeDatum {
  x: number;
  y: number;
}

export type SimLink = SimulationLinkDatum<SimNode>;

/**
 * Seed node positions on a ring centred on the origin and derive links.
 * Tree mode draws an edge from each node to its `parent_id`; contacts mode
 * uses the explicit `edges`. Dangling endpoints are dropped.
 */
export const ROOT_NODE_ID = '__root__';

export function buildGraph(
  nodes: GraphNode[],
  edges: GraphEdge[],
  mode: GraphMode
): { simNodes: SimNode[]; links: SimLink[] } {
  const ids = new Set(nodes.map(n => n.id));

  // Synthetic master root at the origin — all source nodes fan out from it.
  const rootNode: SimNode = { kind: 'root', id: ROOT_NODE_ID, label: 'Memory', x: 0, y: 0 };

  const simNodes: SimNode[] = [rootNode];
  for (let i = 0; i < nodes.length; i++) {
    const n = nodes[i];
    const angle = (i / Math.max(1, nodes.length)) * Math.PI * 2;
    const r = 180 + (i % 7) * 14;
    simNodes.push({ ...n, x: Math.cos(angle) * r, y: Math.sin(angle) * r });
  }

  const links: SimLink[] = [];

  // Link every source node to the master root.
  for (const n of nodes) {
    if (n.kind === 'source') {
      links.push({ source: n.id, target: ROOT_NODE_ID });
    }
  }

  if (mode === 'tree') {
    for (const n of nodes) {
      if (!n.parent_id || !ids.has(n.parent_id) || !ids.has(n.id)) continue;
      links.push({ source: n.id, target: n.parent_id });
    }
  } else {
    for (const e of edges) {
      if (!ids.has(e.from) || !ids.has(e.to)) continue;
      links.push({ source: e.from, target: e.to });
    }
  }
  return { simNodes, links };
}

/**
 * A cooled d3-force simulation (call `.tick()` from the render loop). Charge
 * = Coulomb repulsion (Barnes–Hut), link = Hooke spring, plus centring and
 * a soft collide so nodes don't stack.
 */
export function createSimulation(
  simNodes: SimNode[],
  links: SimLink[]
): Simulation<SimNode, SimLink> {
  return forceSimulation(simNodes)
    .force(
      'charge',
      forceManyBody<SimNode>()
        .strength(n => {
          if (n.kind === 'root') return -650;
          if (n.kind === 'source') return -280;
          return -140;
        })
        .distanceMax(300)
    )
    .force(
      'link',
      forceLink<SimNode, SimLink>(links)
        .id(d => d.id)
        .distance(link => {
          const src = link.source as SimNode;
          const tgt = link.target as SimNode;
          if (src.kind === 'root' || tgt.kind === 'root') return 90;
          if (src.kind === 'source' || tgt.kind === 'source') return 40;
          return 22;
        })
        .strength(0.7)
    )
    .force('center', forceCenter(0, 0).strength(0.12))
    .force(
      'collide',
      forceCollide<SimNode>().radius(n => {
        if (n.kind === 'root') return 80;
        if (n.kind === 'source') return 40;
        return nodeRadius(n) + 2;
      })
    )
    .stop();
}

/**
 * Nearest node whose disc (radius + slop) contains the point, or null.
 * Linear scan — trivial at the 1000-node cap and only runs on pointer
 * events, never per frame.
 */
export function pickNode(simNodes: SimNode[], x: number, y: number, slop = 4): SimNode | null {
  let best: SimNode | null = null;
  let bestD = Infinity;
  for (const n of simNodes) {
    const r = nodeRadius(n) + slop;
    const dx = n.x - x;
    const dy = n.y - y;
    const d = dx * dx + dy * dy;
    if (d <= r * r && d < bestD) {
      bestD = d;
      best = n;
    }
  }
  return best;
}

/** Does the renderer have a usable WebGL context? Drives Pixi-vs-SVG. */
export function supportsWebGL(): boolean {
  if (typeof document === 'undefined') return false;
  try {
    const canvas = document.createElement('canvas');
    return !!(
      canvas.getContext('webgl2') ||
      canvas.getContext('webgl') ||
      canvas.getContext('experimental-webgl')
    );
  } catch {
    return false;
  }
}
