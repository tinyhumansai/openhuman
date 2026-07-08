/**
 * graphDiff (Phase 5c) — a node-level diff between the current canvas draft and
 * an agent-proposed `WorkflowGraph`, used to paint the copilot's diff overlay:
 * added nodes are highlighted, removed nodes are ghosted.
 *
 * Diff is by node `id`. The `workflow_builder` returns a full revised graph
 * (`revise_workflow` echoes the whole `WorkflowGraph`), so a node the user asked
 * to keep retains its id across a revision; genuinely new nodes get fresh ids
 * and dropped nodes disappear from the proposed set. That makes id-set
 * membership a faithful added/removed signal without heuristic matching.
 */
import type { WorkflowGraph } from './types';

export interface GraphDiff {
  /** Node ids present in the proposed graph but not the current one. */
  addedNodeIds: Set<string>;
  /** Node ids present in the current graph but dropped from the proposed one. */
  removedNodeIds: Set<string>;
  /** True when neither set is empty — i.e. the proposal changes the node set. */
  hasChanges: boolean;
}

function nodeIds(graph: WorkflowGraph | null | undefined): Set<string> {
  const ids = new Set<string>();
  for (const node of graph?.nodes ?? []) {
    if (node && typeof node.id === 'string') ids.add(node.id);
  }
  return ids;
}

/**
 * Compute the node-level diff from `current` to `proposed`. Safe against
 * missing/partial graphs — an absent side contributes no ids.
 */
export function diffGraphs(
  current: WorkflowGraph | null | undefined,
  proposed: WorkflowGraph | null | undefined
): GraphDiff {
  const currentIds = nodeIds(current);
  const proposedIds = nodeIds(proposed);

  const addedNodeIds = new Set<string>();
  for (const id of proposedIds) {
    if (!currentIds.has(id)) addedNodeIds.add(id);
  }
  const removedNodeIds = new Set<string>();
  for (const id of currentIds) {
    if (!proposedIds.has(id)) removedNodeIds.add(id);
  }

  return {
    addedNodeIds,
    removedNodeIds,
    hasChanges: addedNodeIds.size > 0 || removedNodeIds.size > 0,
  };
}

/**
 * Build the graph shown DURING preview: the proposed graph, plus any removed
 * nodes (and the edges touching them) carried over from `current` so they can
 * be rendered ghosted instead of vanishing. Accepting the proposal drops these;
 * rejecting reverts to `current`.
 */
export function buildPreviewGraph(
  current: WorkflowGraph,
  proposed: WorkflowGraph,
  removedNodeIds: Set<string>
): WorkflowGraph {
  if (removedNodeIds.size === 0) return proposed;
  const ghostNodes = current.nodes.filter(n => removedNodeIds.has(n.id));
  const proposedNodeIds = new Set(proposed.nodes.map(n => n.id));
  const combinedNodeIds = new Set([...proposedNodeIds, ...removedNodeIds]);
  // Keep current edges that touch a ghosted node and whose other end still
  // exists in the combined view, so a removed node still shows its wiring.
  const ghostEdges = current.edges.filter(
    e =>
      (removedNodeIds.has(e.from_node) || removedNodeIds.has(e.to_node)) &&
      combinedNodeIds.has(e.from_node) &&
      combinedNodeIds.has(e.to_node)
  );
  return {
    ...proposed,
    nodes: [...proposed.nodes, ...ghostNodes],
    edges: [...proposed.edges, ...ghostEdges],
  };
}
