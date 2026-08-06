/**
 * Step-number context — supplies each {@link FlowNodeComponent} card its
 * 1-based execution-order index ("3. Fetch Unread Emails").
 *
 * Deliberately a context rather than a field on React Flow's node `data`, for
 * the same reason {@link CanvasActions} is: `data` is part of the serialized
 * graph, and a number derived from the graph's *shape* has to be recomputed
 * whenever that shape changes.
 *
 * The editable canvas holds its graph in `useNodesState`/`useEdgesState` and
 * mutates it in place — `addNode` builds a node with `createFlowNode`,
 * `onConnect` only calls `setEdges` — so `workflowGraphToXyflow` runs once at
 * mount and never again. A number baked in at adapt time would therefore be
 * absent on any node added mid-edit and stale on every other node the moment a
 * connection changed, until a save or remount. Deriving from the live arrays
 * makes that whole class of staleness unrepresentable.
 *
 * The provider computes one `Map` per nodes/edges change; cards read a single
 * entry. Absent provider (or absent node) yields `undefined`, and the card
 * simply renders no number.
 */
import { createContext, useContext } from 'react';

/** Node id → 1-based execution-order index. */
export const StepNumberContext = createContext<ReadonlyMap<string, number> | null>(null);

/** This node's execution-order index, or `undefined` when unnumbered. */
export function useStepNumber(nodeId: string): number | undefined {
  return useContext(StepNumberContext)?.get(nodeId);
}
