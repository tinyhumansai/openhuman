/**
 * Pure conversion between the tinyflows `WorkflowGraph` wire model
 * (`./types.ts`) and `@xyflow/react`'s `Node`/`Edge` shapes, plus a
 * dependency-free auto-layout for graphs saved without canvas positions.
 *
 * No React here — kept pure so it's trivially unit-testable and reusable by
 * both the read-only canvas (issue B5b.1, `FlowCanvas.tsx`) and the future
 * editable canvas (B5b.2+, which will call `xyflowToWorkflowGraph` on save).
 *
 * Deviation from the B5b.1 plan sketch: `autoLayout` takes `(nodes, edges)`,
 * not `(nodes)` alone. A BFS-from-trigger layer assignment is only
 * meaningful with the edges to walk — a node-only signature would have
 * nothing to traverse and could only ever produce a flat single row.
 */
import type { Edge, Node } from '@xyflow/react';
import createDebug from 'debug';

import type { NodeKind, Port, WorkflowEdge, WorkflowGraph, WorkflowNode } from './types';

const log = createDebug('flows:graphAdapter');

/** The `nodeTypes` key every flow node renders as (see `FlowCanvas.tsx`). */
export const FLOW_NODE_TYPE = 'flowNode';

/**
 * Data carried by every xyflow node's `data` prop. Beyond the raw `kind` /
 * `name` / `config` / `ports` the plan calls for, this also carries the
 * *effective* input/output port names — computed from a union of the node's
 * declared `ports` (output-only, per `types.ts`'s module doc) and whatever
 * port names its edges actually reference — since a `switch` node's live
 * case ports are computed at runtime from config and are not guaranteed to
 * be declared in `ports` at all. `FlowNodeComponent` renders one `Handle`
 * per entry so no wired connection is ever left dangling with no handle to
 * land on.
 */
export interface FlowNodeData extends Record<string, unknown> {
  kind: NodeKind;
  /** `Node.type_version` — carried through so `xyflowToWorkflowGraph` doesn't
   * silently downgrade a node saved with a non-default config version. */
  type_version?: number;
  name: string;
  config: Record<string, unknown>;
  ports: Port[];
  /** Effective input port names, derived from incoming edges (`['main']` if none). */
  inputPorts: string[];
  /** Effective output port names: declared `ports` ∪ outgoing edges' `from_port` (`['main']` if neither). */
  outputPorts: string[];
}

export type FlowNode = Node<FlowNodeData>;
export type FlowEdge = Edge;

interface Point {
  x: number;
  y: number;
}

const DEFAULT_PORT = 'main';
// Sized against the node card, which is 224px wide and ~76px tall (title band
// + summary): 240 leaves a 16px gutter between siblings, and 132 leaves ~56px
// of visible edge between layers — enough for the drop plus a `condition`'s
// `true` / `false` labels, which hang ~20px below their card. These were 280 /
// 160, set when the card was 264px wide and its ports sat on the left and
// right edges.
const LAYOUT_COLUMN_WIDTH = 240;
const LAYOUT_ROW_HEIGHT = 132;

/**
 * Stable, collision-free xyflow edge id for one `WorkflowEdge`. Node ids and
 * port names are free-form strings that may themselves contain `-`, so a
 * plain `${a}-${b}-${c}-${d}` join can collide (e.g. node `"a-b"`/port `"c"`
 * targeting node `"d"`/port `"e"` produces the same joined string as node
 * `"a"`/port `"b-c"` targeting the same target) — and React Flow requires
 * every edge id to be unique. `JSON.stringify` on the 4-tuple escapes any
 * embedded delimiter-like characters and round-trips distinct tuples to
 * distinct strings.
 */
export function edgeId(edge: WorkflowEdge): string {
  return JSON.stringify([edge.from_node, edge.from_port, edge.to_node, edge.to_port]);
}

/** Unique, order-preserving string list. */
function dedupe(values: string[]): string[] {
  return Array.from(new Set(values));
}

/**
 * Effective input port names for `node`: every distinct `to_port` an edge
 * uses to target it, defaulting to `['main']` when nothing targets it (the
 * common case — a plain single-input node, or a trigger with no input at
 * all, still gets a default handle so the canvas reads consistently).
 */
function effectiveInputPorts(node: WorkflowNode, edges: WorkflowEdge[]): string[] {
  const wired = edges.filter(e => e.to_node === node.id).map(e => e.to_port || DEFAULT_PORT);
  return wired.length > 0 ? dedupe(wired) : [DEFAULT_PORT];
}

/**
 * Effective output port names for `node`: the union of its declared `ports`
 * (output-only per the tinyflows model) and every distinct `from_port` an
 * edge uses leaving it — covering dynamically-cased nodes (e.g. `switch`)
 * whose live ports aren't necessarily declared. Defaults to `['main']` when
 * neither source yields anything.
 */
function effectiveOutputPorts(node: WorkflowNode, edges: WorkflowEdge[]): string[] {
  const declared = node.ports.map(p => p.name);
  const wired = edges.filter(e => e.from_node === node.id).map(e => e.from_port || DEFAULT_PORT);
  const combined = dedupe([...declared, ...wired]);
  return combined.length > 0 ? combined : [DEFAULT_PORT];
}

/**
 * Converts a `WorkflowGraph` into xyflow's `{ nodes, edges }` shape for
 * read-only rendering. Nodes missing a saved `position` are laid out via
 * {@link autoLayout} (BFS depth from the trigger). Edge ids/handles are
 * derived directly from the graph's `from_node`/`from_port`/`to_node`/`to_port`.
 */
export function workflowGraphToXyflow(graph: WorkflowGraph): {
  nodes: FlowNode[];
  edges: FlowEdge[];
} {
  log('workflowGraphToXyflow: nodes=%d edges=%d', graph.nodes.length, graph.edges.length);

  const laidOut = autoLayout(graph.nodes, graph.edges);

  const nodes: FlowNode[] = graph.nodes.map(node => {
    const position = node.position ?? laidOut.get(node.id) ?? { x: 0, y: 0 };
    return {
      id: node.id,
      type: FLOW_NODE_TYPE,
      position,
      data: {
        kind: node.kind,
        type_version: node.type_version,
        name: node.name,
        config: node.config ?? {},
        ports: node.ports,
        inputPorts: effectiveInputPorts(node, graph.edges),
        outputPorts: effectiveOutputPorts(node, graph.edges),
      },
    };
  });

  const edges: FlowEdge[] = graph.edges.map(edge => ({
    id: edgeId(edge),
    source: edge.from_node,
    target: edge.to_node,
    sourceHandle: edge.from_port,
    targetHandle: edge.to_port,
  }));

  log(
    'workflowGraphToXyflow: produced %d xyflow nodes, %d xyflow edges',
    nodes.length,
    edges.length
  );
  return { nodes, edges };
}

/**
 * A React-Flow connection candidate (what `onConnect` / `isValidConnection`
 * receive). `sourceHandle`/`targetHandle` are the `Handle` ids — i.e. the
 * effective port names `FlowNodeComponent` renders — and may be `null` when a
 * node exposes a single unnamed handle, in which case they default to `main`.
 */
interface FlowConnectionCandidate {
  source?: string | null;
  target?: string | null;
  sourceHandle?: string | null;
  targetHandle?: string | null;
}

/**
 * Port-aware validity check for a candidate connection on the editable canvas.
 * Rejects (returns `false`) when:
 *  - either endpoint is missing;
 *  - the connection is a self-loop (`source === target`) — tinyflows graphs
 *    are DAG-ish and a node wiring to itself is never meaningful here;
 *  - either endpoint node isn't in `nodes`;
 *  - the source handle isn't one of the source node's effective *output*
 *    ports, or the target handle isn't one of the target node's effective
 *    *input* ports (reusing the same `inputPorts`/`outputPorts` the canvas
 *    already derived in {@link workflowGraphToXyflow} / {@link createFlowNode});
 *  - an identical edge (same 4-tuple) already exists in `edges` — React Flow
 *    would happily add a duplicate otherwise.
 *
 * Pure and dependency-free so both `<ReactFlow isValidConnection>` (live drag
 * feedback) and `onConnect` (the commit) can share one source of truth, and so
 * it's trivially unit-testable.
 */
export function isValidFlowConnection(
  connection: FlowConnectionCandidate,
  nodes: FlowNode[],
  edges: FlowEdge[] = []
): boolean {
  const { source, target } = connection;
  if (!source || !target) {
    log('isValidFlowConnection: reject — missing endpoint');
    return false;
  }
  if (source === target) {
    log('isValidFlowConnection: reject — self-loop on %s', source);
    return false;
  }
  const sourceNode = nodes.find(n => n.id === source);
  const targetNode = nodes.find(n => n.id === target);
  if (!sourceNode || !targetNode) {
    log('isValidFlowConnection: reject — endpoint node not found');
    return false;
  }
  const sourceHandle = connection.sourceHandle || DEFAULT_PORT;
  const targetHandle = connection.targetHandle || DEFAULT_PORT;
  if (!sourceNode.data.outputPorts.includes(sourceHandle)) {
    log('isValidFlowConnection: reject — %s has no output port %s', source, sourceHandle);
    return false;
  }
  if (!targetNode.data.inputPorts.includes(targetHandle)) {
    log('isValidFlowConnection: reject — %s has no input port %s', target, targetHandle);
    return false;
  }
  const duplicate = edges.some(
    e =>
      e.source === source &&
      e.target === target &&
      (e.sourceHandle || DEFAULT_PORT) === sourceHandle &&
      (e.targetHandle || DEFAULT_PORT) === targetHandle
  );
  if (duplicate) {
    log('isValidFlowConnection: reject — duplicate edge');
    return false;
  }
  return true;
}

/**
 * Stable {@link edgeId} for a live React Flow `Connection` candidate (what
 * `onConnect` receives once {@link isValidFlowConnection} has approved it),
 * applying the same `main` port default used throughout this module.
 *
 * `EditableFlowCanvas`'s `onConnect` must pass this to `addEdge` explicitly —
 * left to its own default, `addEdge` mints React Flow's built-in
 * `${source}${sourceHandle}-${target}${targetHandle}` id, which is exactly
 * the plain-concatenation scheme {@link edgeId} exists to avoid (see its
 * doc). Without this, an edge created by drawing a connection on the canvas
 * gets a different id than the same edge would get after a reload (which
 * goes through {@link workflowGraphToXyflow} → {@link edgeId}), and two
 * connections that collide under plain concatenation would produce duplicate
 * React keys.
 */
export function connectionEdgeId(connection: FlowConnectionCandidate): string {
  return edgeId({
    from_node: connection.source ?? '',
    from_port: connection.sourceHandle || DEFAULT_PORT,
    to_node: connection.target ?? '',
    to_port: connection.targetHandle || DEFAULT_PORT,
  });
}

/**
 * Declared output `ports` a freshly-added node of `kind` needs at creation
 * time, for kinds whose runtime routing is fixed and NOT derivable from
 * config or wired edges. A `condition` node always routes through `true`/
 * `false` (`vendor/tinyflows/src/nodes/control_flow/condition.rs`), but the
 * config drawer has no port editor — so unlike `switch` (whose case ports
 * are config-driven and materialize once the author wires an edge, per
 * {@link effectiveOutputPorts}'s doc comment), a new `condition` node must be
 * seeded with both ports up front or its second branch is never wireable
 * from the canvas. A `loop` node is the same shape: it always routes through
 * `body`/`done` (`vendor/tinyflows/src/nodes/control_flow/loop_node.rs`), and
 * the canvas has no port editor to derive them from later — without seeding
 * both here, a palette-added loop renders a single `main` output and neither
 * required loop edge (back to the loop head, and onward past it) can be
 * wired.
 */
function defaultPortsForKind(kind: NodeKind): Port[] {
  if (kind === 'condition') {
    return [{ name: 'true' }, { name: 'false' }];
  }
  if (kind === 'loop') {
    return [{ name: 'body' }, { name: 'done' }];
  }
  return [];
}

/**
 * Build a fresh xyflow node for a palette-added `kind` at `position`. Newly
 * dropped nodes start with a single default `main` input handle (no wired
 * edges yet) plus whatever {@link defaultPortsForKind} declares for `kind` —
 * empty for most kinds, which fall back to the single default `main` output
 * handle exactly as {@link effectiveInputPorts}/{@link effectiveOutputPorts}
 * would derive for a node with no edges yet — so it round-trips cleanly
 * through {@link xyflowToWorkflowGraph} and immediately accepts connections.
 * `id` must be unique within the canvas; `name` defaults to `kind` when
 * omitted.
 */
export function createFlowNode(
  kind: NodeKind,
  position: Point,
  id: string,
  name?: string
): FlowNode {
  const ports = defaultPortsForKind(kind);
  const outputPorts = ports.length > 0 ? ports.map(p => p.name) : [DEFAULT_PORT];
  return {
    id,
    type: FLOW_NODE_TYPE,
    position,
    data: { kind, name: name ?? kind, config: {}, ports, inputPorts: [DEFAULT_PORT], outputPorts },
  };
}

/** Metadata not carried by xyflow nodes/edges, needed to reassemble a full `WorkflowGraph`. */
export interface WorkflowGraphMeta {
  schema_version: number;
  id?: string | null;
  name: string;
}

/**
 * Reverses {@link workflowGraphToXyflow}: reassembles a `WorkflowGraph` from
 * xyflow's `nodes`/`edges` plus the graph-level metadata xyflow doesn't
 * carry (`schema_version`/`id`/`name`). Defined now — read-only B5b.1 has no
 * editor UI to call it from yet — so it's co-located with its inverse ahead
 * of B5b.4's save path, and so its round-trip behavior is locked in by tests
 * from day one.
 *
 * Every field `workflowGraphToXyflow` carries over from the source
 * `WorkflowNode` — including `type_version` — round-trips back out here;
 * `node.position` always comes out concrete (never `undefined`) since by the
 * time a node exists on an editable canvas it has a real position, which
 * matches a freshly-authored node too. `inputPorts`/`outputPorts` are
 * canvas-only derived fields (see `FlowNodeData`'s doc comment) and are
 * intentionally not written back — only the *declared* `ports` round-trip.
 */
export function xyflowToWorkflowGraph(
  nodes: FlowNode[],
  edges: FlowEdge[],
  meta: WorkflowGraphMeta
): WorkflowGraph {
  const workflowNodes: WorkflowNode[] = nodes.map(node => ({
    id: node.id,
    kind: node.data.kind,
    type_version: node.data.type_version,
    name: node.data.name,
    config: node.data.config,
    ports: node.data.ports,
    position: { x: node.position.x, y: node.position.y },
  }));

  const workflowEdges: WorkflowEdge[] = edges.map(edge => ({
    from_node: edge.source,
    from_port: edge.sourceHandle ?? DEFAULT_PORT,
    to_node: edge.target,
    to_port: edge.targetHandle ?? DEFAULT_PORT,
  }));

  return {
    schema_version: meta.schema_version,
    id: meta.id,
    name: meta.name,
    nodes: workflowNodes,
    edges: workflowEdges,
  };
}

/**
 * Round-trips `graph` through {@link workflowGraphToXyflow} /
 * {@link xyflowToWorkflowGraph} — the exact conversion the editable canvas
 * itself performs on mount — so a caller comparing `graph` against a LATER
 * canvas serialization (to decide "is this dirty?") compares like with like.
 *
 * F-m3 fix: a graph saved without per-node `position` (e.g. authored by the
 * agent, which never sets one) gets concrete auto-layout coordinates baked in
 * the moment the canvas mounts it (`workflowGraphToXyflow`'s `autoLayout`
 * fallback) — `EditableFlowCanvas`'s mount-time `onGraphChange` reports that
 * position-filled graph immediately. A dirty check that diffs the canvas's
 * report against the RAW, positionless server graph reads that fill-in as an
 * edit and shows "Unsaved" with zero real changes. Normalizing the baseline
 * through this same round-trip before comparing eliminates that false
 * positive, since both sides then agree on where every node landed.
 */
export function normalizeWorkflowGraphForDirtyCheck(
  graph: WorkflowGraph,
  meta: WorkflowGraphMeta
): WorkflowGraph {
  const { nodes, edges } = workflowGraphToXyflow(graph);
  return xyflowToWorkflowGraph(nodes, edges, meta);
}

/**
 * Assigns a `{x, y}` position to every node in `nodes`, via a simple BFS
 * layering over `edges`: `y = depth * 132`, `x = column * 240` where
 * `column` is the node's index within its depth layer (assigned in
 * declaration order). Roots are nodes with no incoming edge (normally just
 * the trigger); disconnected sub-graphs and cycles still terminate — any
 * node the BFS doesn't reach is appended one layer past the deepest reached
 * node, so nothing is ever silently dropped. No extra dependency (no
 * `dagre`) — good enough for a read-only first render; a real editor can
 * offer manual repositioning later.
 *
 * Returns a `Map<nodeId, Point>` for every node passed in (not just the ones
 * missing a saved position), so callers can use it as a uniform fallback
 * source; `workflowGraphToXyflow` only consults it for nodes lacking
 * `position`.
 */
export function autoLayout(nodes: WorkflowNode[], edges: WorkflowEdge[]): Map<string, Point> {
  const positions = new Map<string, Point>();
  if (nodes.length === 0) return positions;

  const nodeIds = new Set(nodes.map(n => n.id));
  const incoming = new Map<string, number>(nodes.map(n => [n.id, 0]));
  const adjacency = new Map<string, string[]>(nodes.map(n => [n.id, []]));

  for (const edge of edges) {
    // Ignore edges referencing ids outside this node set — defensive only;
    // a validated graph never has these, but layout should never throw.
    if (!nodeIds.has(edge.from_node) || !nodeIds.has(edge.to_node)) continue;
    adjacency.get(edge.from_node)?.push(edge.to_node);
    incoming.set(edge.to_node, (incoming.get(edge.to_node) ?? 0) + 1);
  }

  // Roots: nodes with no incoming edge (usually just the trigger), in
  // declaration order for determinism. Falls back to every node if the
  // graph has no such root (a cycle, or every node has an incoming edge).
  const roots = nodes.filter(n => (incoming.get(n.id) ?? 0) === 0);
  const startIds = (roots.length > 0 ? roots : nodes).map(n => n.id);

  const depth = new Map<string, number>();
  const queue: string[] = [];
  for (const id of startIds) {
    if (depth.has(id)) continue;
    depth.set(id, 0);
    queue.push(id);
  }

  let head = 0;
  while (head < queue.length) {
    const id = queue[head++];
    const currentDepth = depth.get(id) ?? 0;
    for (const nextId of adjacency.get(id) ?? []) {
      if (depth.has(nextId)) continue;
      depth.set(nextId, currentDepth + 1);
      queue.push(nextId);
    }
  }

  // Any node unreached by the BFS (disconnected sub-graph, or a cycle with
  // no zero-in-degree entry point) still gets a depth so it renders
  // somewhere rather than being silently dropped.
  let maxDepth = 0;
  for (const d of depth.values()) maxDepth = Math.max(maxDepth, d);
  for (const node of nodes) {
    if (!depth.has(node.id)) depth.set(node.id, ++maxDepth);
  }

  const columnByDepth = new Map<number, number>();
  for (const node of nodes) {
    const d = depth.get(node.id) ?? 0;
    const column = columnByDepth.get(d) ?? 0;
    columnByDepth.set(d, column + 1);
    positions.set(node.id, { x: column * LAYOUT_COLUMN_WIDTH, y: d * LAYOUT_ROW_HEIGHT });
  }

  return positions;
}

/**
 * Assigns each node a 1-based execution-order index for display ("3. Fetch
 * Unread Emails").
 *
 * Uses the same breadth-first walk as {@link autoLayout} — chain-starting roots
 * (no incoming edge but at least one outgoing, normally just the trigger)
 * first, then each successor as it is reached — so a card's number always
 * agrees with its position in the laid-out graph rather than with declaration
 * order, which is arbitrary for a graph the agent authored.
 *
 * A DAG has no single "correct" ordering once it branches: two parallel
 * branches genuinely run concurrently, so any numbering imposes an order that
 * execution does not guarantee. BFS is chosen because it numbers breadth-first
 * across a fan-out (both branches' first steps get adjacent numbers) rather
 * than running one branch to its end before starting the next, which is what a
 * depth-first walk would do and reads as wrong beside a two-column layout.
 *
 * Independent chains are kept contiguous rather than interleaved: each root's
 * component is drained completely before the next root is seeded. Otherwise
 * `a → b` plus `c → d` would number a=1, c=2, b=3, d=4, and drawing a second
 * chain would renumber the first one's steps.
 *
 * Every node gets a number, including ones the walk cannot reach — a node not
 * yet wired up, or a cycle with no zero-in-degree entry. Those are appended in
 * declaration order after the reachable ones, so no card renders without an
 * index and, crucially, adding disconnected work to the canvas never renumbers
 * the flow it has not joined yet.
 */
export function stepNumbers(nodes: WorkflowNode[], edges: WorkflowEdge[]): Map<string, number> {
  return stepNumbersFor(
    nodes.map(n => n.id),
    edges.map(e => [e.from_node, e.to_node])
  );
}

/**
 * {@link stepNumbers} for the xyflow shape — the live editing state.
 *
 * The editable canvas holds its graph in `useNodesState`/`useEdgesState` and
 * mutates it directly (`addNode` builds a node with `createFlowNode`,
 * `onConnect` only calls `setEdges`), so `workflowGraphToXyflow` runs once at
 * mount and never again. Numbering must therefore be derived from the live
 * arrays on every change rather than baked into node `data` at adapt time — a
 * node added mid-edit has no adapter-supplied number at all, and every existing
 * number goes stale the moment the topology changes.
 */
export function stepNumbersForFlow(nodes: FlowNode[], edges: FlowEdge[]): Map<string, number> {
  return stepNumbersFor(
    nodes.map(n => n.id),
    edges.map(e => [e.source, e.target])
  );
}

/**
 * Shared BFS behind both public entry points, over bare ids and `[from, to]`
 * pairs so the two graph shapes cannot drift in ordering behaviour.
 */
function stepNumbersFor(ids: string[], pairs: Array<[string, string]>): Map<string, number> {
  const order = new Map<string, number>();
  if (ids.length === 0) return order;

  const known = new Set(ids);
  const incoming = new Map<string, number>(ids.map(id => [id, 0]));
  const adjacency = new Map<string, string[]>(ids.map(id => [id, []]));
  for (const [from, to] of pairs) {
    if (!known.has(from) || !known.has(to)) continue;
    adjacency.get(from)?.push(to);
    incoming.set(to, (incoming.get(to) ?? 0) + 1);
  }

  // Seed only with roots that actually start a chain — in-degree 0 AND at
  // least one outgoing edge. A node with no edges at all is *not* step 2 just
  // because it has no incoming one: on the editable canvas a node dropped from
  // the palette is unwired for as long as it takes to connect it, and seeding
  // it as a root would renumber the entire flow underneath the user each time
  // they added one. Such nodes fall through to the trailing pass below and are
  // numbered after the wired flow instead.
  const roots = ids.filter(
    id => (incoming.get(id) ?? 0) === 0 && (adjacency.get(id)?.length ?? 0) > 0
  );

  const seen = new Set<string>();
  let next = 1;

  // One component at a time: drain each root's BFS completely before seeding
  // the next. Seeding every root at once would interleave independent chains —
  // for `a → b` and `c → d` that yields a=1, c=2, b=3, d=4, so drawing a second
  // chain on the canvas renumbers the first one's steps. That is the same
  // renumbering-underneath-the-user problem the root filter above avoids for
  // lone nodes, and it would contradict this function's contract that
  // disconnected work is numbered *after* the flow it has not joined.
  //
  // BFS still applies *within* a component, so a fan-out's sibling branches
  // keep adjacent numbers; only whole components are kept contiguous.
  for (const root of roots) {
    if (seen.has(root)) continue;
    const queue: string[] = [root];
    seen.add(root);
    let head = 0;
    while (head < queue.length) {
      const id = queue[head++];
      order.set(id, next++);
      for (const nextId of adjacency.get(id) ?? []) {
        if (seen.has(nextId)) continue;
        seen.add(nextId);
        queue.push(nextId);
      }
    }
  }

  // Everything the walk never reached: unwired nodes, disconnected sub-graphs,
  // and cycles with no zero-in-degree entry. Numbered in declaration order so
  // every card still shows an index.
  for (const id of ids) {
    if (!order.has(id)) order.set(id, next++);
  }
  return order;
}
