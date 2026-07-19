/**
 * FlowCanvas — the Workflow Canvas view. Two modes behind one entry point:
 *
 *  - **read-only** (default, issue B5b.1): renders a saved flow's
 *    `WorkflowGraph` (already converted to xyflow's shape by `graphAdapter.ts`)
 *    with a minimap, zoom/pan controls, and a dotted background. Every
 *    interaction that would mutate the graph is disabled.
 *  - **editable** (`editable`, issue B5b.2 / Phase 3a): delegates to
 *    {@link EditableFlowCanvas}, which lifts nodes/edges into controlled state
 *    and wires drag/connect/add/delete/save on top.
 *
 * The `editable` prop defaults to `false` so every existing read-only consumer
 * (the `/flows/:id` viewer) keeps its exact behavior — only the builder opts in.
 */
import {
  Background,
  BackgroundVariant,
  Controls,
  MiniMap,
  ReactFlow,
  type Viewport,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { forwardRef, memo, useMemo } from 'react';

import {
  FLOW_NODE_TYPE,
  type FlowEdge,
  type FlowNode,
  type WorkflowGraphMeta,
} from '../../../lib/flows/graphAdapter';
import type { WorkflowGraph } from '../../../lib/flows/types';
import EditableFlowCanvas, {
  type EditableFlowCanvasHandle,
  type EditorSaveMeta,
} from './EditableFlowCanvas';
import './flowCanvasStyles.css';
import FlowNodeComponent from './FlowNodeComponent';

export interface FlowCanvasProps {
  nodes: FlowNode[];
  edges: FlowEdge[];
  /**
   * Enable the editable builder (drag/connect/add/delete/save). Defaults to
   * `false` — the read-only viewer that ships everywhere else stays intact.
   */
  editable?: boolean;
  /** Graph-level metadata needed to re-serialize on save (editable only). */
  meta?: WorkflowGraphMeta;
  /** Save callback: receives the live canvas as a `WorkflowGraph` (editable only). */
  onSave?: (graph: WorkflowGraph) => void | Promise<void>;
  /** Reports the editable draft's dirty state so the host can guard navigation (editable only). */
  onDirtyChange?: (dirty: boolean) => void;
  /** Active run id (== thread_id) to overlay live per-node status on the canvas (editable only, Phase 3e). */
  activeRunId?: string | null;
  /** Reports the live graph on every edit so the copilot has the current draft (editable only, Phase 5c). */
  onGraphChange?: (graph: WorkflowGraph) => void;
  /** Node ids the copilot proposal adds — ringed as a diff highlight (editable only, Phase 5c). */
  addedNodeIds?: ReadonlySet<string>;
  /** Node ids the copilot proposal removes — ghosted (editable only, Phase 5c). */
  removedNodeIds?: ReadonlySet<string>;
  /** Disable Save while a copilot proposal is under review (editable only, Phase 5c). */
  saveDisabled?: boolean;
  /**
   * Seed the editable canvas as already-dirty at mount (editable only, Phase
   * 5c fix) — see `EditableFlowCanvas`'s `initialDirty` doc comment. The host
   * computes this by diffing the incoming graph against its own
   * last-persisted snapshot so a `key`-remount (e.g. accepting a copilot
   * proposal) doesn't silently clear unsaved state.
   */
  initialDirty?: boolean;
  /**
   * Show the drag-and-drop node palette ("Legend") overlay (editable only).
   * Driven by the canvas header's Copilot | Legend toggle. Defaults to `true`
   * so non-toggling consumers keep the palette visible.
   */
  showPalette?: boolean;
  /** Reports Save-button state up so the host header can render Save/Discard. */
  onSaveMetaChange?: (meta: EditorSaveMeta) => void;
  /**
   * The viewport (pan/zoom) to restore on mount (editable only, F4/F5 fix) —
   * see `EditableFlowCanvas`'s `savedViewport` doc comment. Not threaded to
   * the read-only viewer, which keeps its unconditional `fitView`.
   */
  savedViewport?: Viewport | null;
  /** Fired on every viewport change (editable only, F4/F5 fix). */
  onViewportChange?: (viewport: Viewport) => void;
}

const NODE_TYPES = { [FLOW_NODE_TYPE]: FlowNodeComponent };

// Stable fallback so an omitted `meta` doesn't allocate a new object every
// render — `meta ?? { ... }` inline would defeat EditableFlowCanvas's
// `useMemo(..., [meta])` dependency (a new referentially-distinct object each
// render forces it to re-serialize the whole graph even with no real edit).
const DEFAULT_META: WorkflowGraphMeta = { schema_version: 1, name: '' };

/**
 * Read-only render path — unchanged from B5b.1. Kept a separate component so
 * its `useMemo` hook stays unconditional and the editable path's controlled
 * state hooks never run for a plain viewer.
 */
function ReadonlyFlowCanvas({ nodes, edges }: { nodes: FlowNode[]; edges: FlowEdge[] }) {
  const interactionProps = useMemo(
    () => ({ nodesDraggable: false, nodesConnectable: false, elementsSelectable: false }),
    []
  );

  return (
    <div className="flow-canvas h-full w-full" data-testid="flow-canvas">
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={NODE_TYPES}
        fitView
        panOnScroll
        zoomOnScroll
        {...interactionProps}>
        <Background variant={BackgroundVariant.Dots} gap={16} size={1} />
        <MiniMap pannable zoomable />
        <Controls showInteractive={false} />
      </ReactFlow>
    </div>
  );
}

/**
 * Fills its parent's box (`h-full w-full` — the page decides how tall/wide
 * that is; `FlowCanvasPage` gives it the full panel body).
 */
const FlowCanvas = forwardRef<EditableFlowCanvasHandle, FlowCanvasProps>(
  (
    {
      nodes,
      edges,
      editable = false,
      meta,
      onSave,
      onDirtyChange,
      activeRunId,
      onGraphChange,
      addedNodeIds,
      removedNodeIds,
      saveDisabled,
      initialDirty,
      showPalette = true,
      onSaveMetaChange,
      savedViewport,
      onViewportChange,
    }: FlowCanvasProps,
    ref
  ) => {
    if (editable) {
      return (
        <EditableFlowCanvas
          ref={ref}
          nodes={nodes}
          edges={edges}
          meta={meta ?? DEFAULT_META}
          onSave={onSave}
          onDirtyChange={onDirtyChange}
          activeRunId={activeRunId}
          onGraphChange={onGraphChange}
          addedNodeIds={addedNodeIds}
          removedNodeIds={removedNodeIds}
          saveDisabled={saveDisabled}
          initialDirty={initialDirty}
          showPalette={showPalette}
          onSaveMetaChange={onSaveMetaChange}
          savedViewport={savedViewport}
          onViewportChange={onViewportChange}
        />
      );
    }
    return <ReadonlyFlowCanvas nodes={nodes} edges={edges} />;
  }
);

export default memo(FlowCanvas);
