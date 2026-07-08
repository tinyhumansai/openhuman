/**
 * Canvas actions context — lets a selected {@link FlowNodeComponent} card
 * trigger the editor-level actions (delete this node, validate the graph)
 * without threading callbacks through React Flow's node `data` (which is part
 * of the serialized graph). `EditableFlowCanvas` provides it; the read-only
 * viewer leaves it `null`, so node cards render no actions there.
 */
import { createContext, useContext } from 'react';

export interface CanvasActions {
  /** Remove a single node (and its incident edges) by id. */
  deleteNode: (nodeId: string) => void;
  /** Run a full-graph validation pass. */
  validate: () => void;
  /** True while a validation pass is in flight. */
  validating: boolean;
}

export const CanvasActionsContext = createContext<CanvasActions | null>(null);

/** Returns the canvas actions, or `null` in the read-only viewer. */
export function useCanvasActions(): CanvasActions | null {
  return useContext(CanvasActionsContext);
}
