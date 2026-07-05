/**
 * EditableFlowCanvas — the mutable Workflow Canvas (issue B5b.2 / Phase 3a).
 * Wraps `@xyflow/react`'s `<ReactFlow>` in *controlled* mode: node/edge state
 * is lifted into `useNodesState`/`useEdgesState` seeded once from the incoming
 * graph, so drags, connections, additions, and deletions mutate local state
 * rather than the read-only viewer's static props.
 *
 * What it wires on top of the read-only `FlowCanvas`:
 *  - **drag / move** — `nodesDraggable` on; `onNodesChange` persists positions.
 *  - **connect** — `onConnect` is port-aware: it accepts a new edge only when
 *    {@link isValidFlowConnection} approves it (reusing the canvas's derived
 *    input/output ports), and rejects self-loops, unknown handles, and dupes.
 *  - **delete** — Backspace/Delete removes the selection (React Flow default),
 *    plus an explicit "Delete selected" toolbar button as a discoverable
 *    affordance; deleting a node also drops its incident edges.
 *  - **add** — a {@link NodePalette} inserts any of the 12 node kinds by click
 *    (default cascade position) or drag-drop (under the cursor).
 *  - **save** — a "Save" button serializes the live canvas back to a
 *    `WorkflowGraph` via {@link xyflowToWorkflowGraph} and hands it to `onSave`.
 *    The dirty-guard / persistence call lives one layer up (Phase 3d).
 */
import {
  addEdge,
  Background,
  BackgroundVariant,
  type Connection,
  Controls,
  MiniMap,
  ReactFlow,
  type ReactFlowInstance,
  useEdgesState,
  useNodesState,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import createDebug from 'debug';
import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { FLOW_RUN_NODE_STATUS_CLASS, useFlowRunProgress } from '../../../hooks/useFlowRunProgress';
import { erroredNodeIds } from '../../../lib/flows/flowValidation';
import {
  createFlowNode,
  FLOW_NODE_TYPE,
  type FlowEdge,
  type FlowNode,
  isValidFlowConnection,
  type WorkflowGraphMeta,
  xyflowToWorkflowGraph,
} from '../../../lib/flows/graphAdapter';
import type { NodeKind, WorkflowGraph } from '../../../lib/flows/types';
import { useT } from '../../../lib/i18n/I18nContext';
import { type FlowConnection, listFlowConnections } from '../../../services/api/flowsApi';
import Button from '../../ui/Button';
import './flowCanvasStyles.css';
import FlowNodeComponent from './FlowNodeComponent';
import FlowValidationBanner from './FlowValidationBanner';
import NodeConfigDrawer, { type NodeConfigPatch } from './nodeConfig/NodeConfigDrawer';
import NodePalette, { PALETTE_DND_MIME } from './NodePalette';
import { useFlowValidation } from './useFlowValidation';

const log = createDebug('app:flows:canvas:edit');

const NODE_TYPES = { [FLOW_NODE_TYPE]: FlowNodeComponent };
const DELETE_KEYS = ['Backspace', 'Delete'];

/** Where a click-added palette node lands (canvas coords) before cascade. */
const CLICK_ADD_ORIGIN = { x: 80, y: 80 };
/** Per-click cascade so repeated palette clicks don't stack on one spot. */
const CLICK_ADD_STEP = 32;

export interface EditableFlowCanvasProps {
  nodes: FlowNode[];
  edges: FlowEdge[];
  /** Graph-level metadata xyflow doesn't carry, needed to re-serialize on save. */
  meta: WorkflowGraphMeta;
  /**
   * Called with the current canvas serialized to a `WorkflowGraph` when the
   * user clicks Save. The caller owns the `flows_update` RPC (Phase 3d); this
   * component runs validation and gates Save on hard errors before invoking it.
   * May return a promise — Save awaits it, and only advances the dirty baseline
   * (clearing unsaved state) once it resolves. A rejection surfaces inline.
   */
  onSave?: (graph: WorkflowGraph) => void | Promise<void>;
  /** Fired when a drawn connection is rejected as invalid (for a toast in 3c). */
  onInvalidConnection?: (connection: Connection) => void;
  /**
   * Reports the draft's dirty state (unsaved edits vs the last saved baseline)
   * so the host page can gate navigation-away (Phase 3d).
   */
  onDirtyChange?: (dirty: boolean) => void;
  /**
   * Id of the currently-executing run (== thread_id) to overlay live per-node
   * status on the canvas (Phase 3e). `null`/absent means no run is in flight,
   * so no overlay is drawn. The live feed is best-effort — the durable
   * `flow_runs` row + {@link useFlowRunPoller} remain the source of truth.
   */
  activeRunId?: string | null;
  /**
   * Reports the canvas's live graph on every edit (Phase 5c) so the host can
   * feed the current draft to the copilot as context and diff a proposal
   * against it. Fires with the same serialization Save uses.
   */
  onGraphChange?: (graph: WorkflowGraph) => void;
  /**
   * Node ids the copilot's pending proposal ADDS — ringed sage as a diff
   * highlight (Phase 5c). Empty/absent when not previewing a proposal.
   */
  addedNodeIds?: ReadonlySet<string>;
  /**
   * Node ids the copilot's pending proposal REMOVES — ghosted (Phase 5c). These
   * nodes are still rendered (carried over by the host) so the removal is
   * visible before Accept/Reject.
   */
  removedNodeIds?: ReadonlySet<string>;
  /**
   * Force-disable Save (Phase 5c) — set while a copilot proposal is under
   * review so the ghosted preview graph can't be persisted; Accept/Reject in
   * the copilot panel is the gate instead.
   */
  saveDisabled?: boolean;
  /**
   * Seed the dirty flag as already-unsaved at mount (Phase 5c fix). This
   * component's dirty baseline is seeded from `nodes`/`edges` at mount, so
   * whenever the host remounts the canvas with a new key (e.g. accepting a
   * copilot proposal, `FlowCanvasPage`'s `canvasVersion` bump) the freshly
   * mounted graph would otherwise instantly read as "clean" even though it
   * was never actually persisted via `onSave` — losing the accepted changes
   * on back/reload instead of gating them behind Save. The host computes
   * this by comparing the incoming graph against its own last-persisted
   * snapshot and passes the result through, independent of any canvas
   * remount.
   */
  initialDirty?: boolean;
}

const EMPTY_ID_SET: ReadonlySet<string> = new Set();

function EditableFlowCanvas({
  nodes: initialNodes,
  edges: initialEdges,
  meta,
  onSave,
  onInvalidConnection,
  onDirtyChange,
  activeRunId = null,
  onGraphChange,
  addedNodeIds = EMPTY_ID_SET,
  removedNodeIds = EMPTY_ID_SET,
  saveDisabled = false,
  initialDirty = false,
}: EditableFlowCanvasProps) {
  const { t } = useT();
  const [nodes, setNodes, onNodesChange] = useNodesState<FlowNode>(initialNodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState<FlowEdge>(initialEdges);
  const rfRef = useRef<ReactFlowInstance<FlowNode, FlowEdge> | null>(null);
  const addCounter = useRef(0);
  const [selectionCount, setSelectionCount] = useState(0);
  // Id of the single selected node whose config the drawer edits (`null` when
  // zero or multiple nodes — or any edge — are selected).
  const [configNodeId, setConfigNodeId] = useState<string | null>(null);
  const [connections, setConnections] = useState<FlowConnection[]>([]);

  // ── Draft / dirty state (Phase 3d) ────────────────────────────────────────
  // The last *saved* snapshot: the graph is "dirty" whenever the live canvas
  // serializes to something different. Seeded from the incoming graph and
  // advanced on every successful Save so post-save the canvas reads clean.
  const [baseline, setBaseline] = useState<{ nodes: FlowNode[]; edges: FlowEdge[] }>(() => ({
    nodes: initialNodes,
    edges: initialEdges,
  }));
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  // Host-computed "already unsaved at mount" override (Phase 5c fix, see
  // `initialDirty`'s doc comment) — cleared once a real Save/Discard
  // resolves this instance's baseline, same as the self-computed `dirty`.
  const [forcedDirty, setForcedDirty] = useState(initialDirty);

  const currentGraph = useMemo(
    () => xyflowToWorkflowGraph(nodes, edges, meta),
    [nodes, edges, meta]
  );
  const currentKey = useMemo(() => JSON.stringify(currentGraph), [currentGraph]);

  // Report the live graph up (Phase 5c) so the copilot always has the current
  // draft to build on. Keyed on `currentKey` so it fires once per real change,
  // not on every render.
  useEffect(() => {
    onGraphChange?.(currentGraph);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentKey]);
  const baselineKey = useMemo(
    () => JSON.stringify(xyflowToWorkflowGraph(baseline.nodes, baseline.edges, meta)),
    [baseline, meta]
  );
  const dirty = forcedDirty || currentKey !== baselineKey;

  // Notify the host page so it can gate navigation-away while dirty.
  useEffect(() => {
    onDirtyChange?.(dirty);
  }, [dirty, onDirtyChange]);

  // ── Validation (Phase 3c) ─────────────────────────────────────────────────
  const { validation, validating, validateNow } = useFlowValidation(currentGraph, currentKey);
  // Only a *present, failed* validation blocks Save — a null result (not yet
  // run, or the RPC failed) fails open, since the server re-validates on update.
  const hasErrors = validation ? !validation.valid : false;

  // Ids named by a hard error, so the canvas can ring the offending node(s).
  const erroredIds = useMemo(
    () =>
      erroredNodeIds(
        validation && !validation.valid ? validation.errors : [],
        nodes.map(n => n.id)
      ),
    [validation, nodes]
  );
  // ── Live run overlay (Phase 3e) ───────────────────────────────────────────
  // Subscribe to the core's per-step progress feed for the active run and map
  // each node id to a live-status ring class. This CLOSES Phase 1's deferred
  // "frontend consumes FlowRunProgress" follow-up. The 2s poller in
  // `useFlowRunPoller` stays as the durable fallback; this just makes it live.
  const runProgress = useFlowRunProgress(activeRunId);

  // Derive the render array (never stored in draft, so it can't dirty the graph):
  // tag errored nodes with the `flow-node-error` class the canvas CSS rings, and
  // overlay each node's live run status (`flow-node-running`/`-success`/`-failed`).
  const hasRunOverlay = Object.keys(runProgress).length > 0;
  const hasDiffOverlay = addedNodeIds.size > 0 || removedNodeIds.size > 0;
  const displayNodes = useMemo(() => {
    if (erroredIds.size === 0 && !hasRunOverlay && !hasDiffOverlay) return nodes;
    return nodes.map(n => {
      const extra: string[] = [];
      if (erroredIds.has(n.id)) extra.push('flow-node-error');
      const runClass = FLOW_RUN_NODE_STATUS_CLASS[runProgress[n.id]];
      if (runClass) extra.push(runClass);
      // Copilot diff overlay (Phase 5c): sage ring on added, ghost on removed.
      if (addedNodeIds.has(n.id)) extra.push('flow-node-added');
      if (removedNodeIds.has(n.id)) extra.push('flow-node-removed');
      if (extra.length === 0) return n;
      return { ...n, className: `${n.className ?? ''} ${extra.join(' ')}`.trim() };
    });
    // `runProgress` is a stable-enough dependency (new object only on a real
    // status change, see the hook's setState guard).
  }, [nodes, erroredIds, runProgress, hasRunOverlay, hasDiffOverlay, addedNodeIds, removedNodeIds]);

  // Load the secret-free credential refs once for the node-config credential
  // picker (http_request / tool_call). Guarded: outside Tauri (or if the RPC
  // fails) the picker just shows its empty state rather than throwing.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const list = await listFlowConnections();
        if (cancelled) return;
        log('connections loaded: count=%d', list.length);
        setConnections(list);
      } catch (err) {
        if (cancelled) return;
        log('connections load failed (non-fatal): %o', err);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const nextNodeId = useCallback((kind: NodeKind): string => {
    // Prefix keeps palette-added ids from ever colliding with loaded graph ids
    // (which are arbitrary backend strings); the counter keeps them unique
    // within a session even for same-kind, same-millisecond clicks.
    return `new-${kind}-${addCounter.current++}`;
  }, []);

  const onConnect = useCallback(
    (connection: Connection) => {
      if (!isValidFlowConnection(connection, nodes, edges)) {
        log('onConnect: rejected %o', connection);
        onInvalidConnection?.(connection);
        return;
      }
      log('onConnect: accepted %o', connection);
      setEdges(current => addEdge(connection, current));
    },
    [nodes, edges, setEdges, onInvalidConnection]
  );

  // Live drag feedback: React Flow calls this while dragging a new connection
  // and paints the target handle valid/invalid before the drop commits.
  const isValidConnection = useCallback(
    (connection: Connection | FlowEdge) =>
      isValidFlowConnection(connection as Connection, nodes, edges),
    [nodes, edges]
  );

  const addNode = useCallback(
    (kind: NodeKind, position: { x: number; y: number }) => {
      const id = nextNodeId(kind);
      const name = t(`flows.nodeKind.${kind}`, kind);
      const node = createFlowNode(kind, position, id, name);
      log('addNode: kind=%s id=%s at %o', kind, id, position);
      setNodes(current => [...current, node]);
    },
    [nextNodeId, setNodes, t]
  );

  const handlePaletteAdd = useCallback(
    (kind: NodeKind) => {
      const step = addCounter.current * CLICK_ADD_STEP;
      addNode(kind, { x: CLICK_ADD_ORIGIN.x + step, y: CLICK_ADD_ORIGIN.y + step });
    },
    [addNode]
  );

  const handleDrop = useCallback(
    (event: React.DragEvent) => {
      event.preventDefault();
      const kind = event.dataTransfer.getData(PALETTE_DND_MIME) as NodeKind;
      if (!kind) return;
      const instance = rfRef.current;
      const position = instance
        ? instance.screenToFlowPosition({ x: event.clientX, y: event.clientY })
        : { ...CLICK_ADD_ORIGIN };
      addNode(kind, position);
    },
    [addNode]
  );

  const handleDragOver = useCallback((event: React.DragEvent) => {
    event.preventDefault();
    event.dataTransfer.dropEffect = 'copy';
  }, []);

  const handleDeleteSelected = useCallback(() => {
    const removedNodeIds = new Set(nodes.filter(n => n.selected).map(n => n.id));
    const removedEdgeIds = new Set(edges.filter(e => e.selected).map(e => e.id));
    if (removedNodeIds.size === 0 && removedEdgeIds.size === 0) return;
    log('deleteSelected: nodes=%d edges=%d', removedNodeIds.size, removedEdgeIds.size);
    setNodes(current => current.filter(n => !removedNodeIds.has(n.id)));
    // Drop explicitly-selected edges AND any edge left dangling by a removed node.
    setEdges(current =>
      current.filter(
        e =>
          !removedEdgeIds.has(e.id) &&
          !removedNodeIds.has(e.source) &&
          !removedNodeIds.has(e.target)
      )
    );
  }, [nodes, edges, setNodes, setEdges]);

  const handleSave = useCallback(async () => {
    // Hard errors block Save (warnings are allowed through). Belt-and-braces:
    // the button is also disabled in this state.
    if (hasErrors) {
      log('save: blocked — graph has validation errors');
      return;
    }
    const graph = xyflowToWorkflowGraph(nodes, edges, meta);
    log('save: nodes=%d edges=%d', graph.nodes.length, graph.edges.length);
    setSaving(true);
    setSaveError(null);
    try {
      await onSave?.(graph);
      // Advance the dirty baseline to the just-saved snapshot so the canvas
      // reads clean (and the nav guard stands down) until the next edit.
      setBaseline({ nodes, edges });
      setForcedDirty(false);
      log('save: succeeded — baseline advanced');
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      log('save: failed err=%o', err);
      setSaveError(message);
    } finally {
      setSaving(false);
    }
  }, [hasErrors, nodes, edges, meta, onSave]);

  // Discard all unsaved edits, resetting the canvas to the last saved baseline.
  const handleDiscard = useCallback(() => {
    log(
      'discard: resetting to baseline nodes=%d edges=%d',
      baseline.nodes.length,
      baseline.edges.length
    );
    setNodes(baseline.nodes);
    setEdges(baseline.edges);
    setConfigNodeId(null);
    setSaveError(null);
    setForcedDirty(false);
  }, [baseline, setNodes, setEdges]);

  const handleValidate = useCallback(() => {
    log('validate: manual trigger');
    void validateNow();
  }, [validateNow]);

  const onSelectionChange = useCallback(
    ({ nodes: selNodes, edges: selEdges }: { nodes: FlowNode[]; edges: FlowEdge[] }) => {
      setSelectionCount(selNodes.length + selEdges.length);
      // Open the config drawer only for an unambiguous single-node selection;
      // any edge in the selection, or 0/2+ nodes, closes it.
      const nextId = selEdges.length === 0 && selNodes.length === 1 ? selNodes[0].id : null;
      log(
        'selectionChange: nodes=%d edges=%d configNode=%s',
        selNodes.length,
        selEdges.length,
        nextId ?? 'none'
      );
      setConfigNodeId(nextId);
    },
    []
  );

  // Apply a name/config edit from the drawer to the live node state (controlled).
  const updateNode = useCallback(
    (nodeId: string, patch: NodeConfigPatch) => {
      log(
        'updateNode: id=%s name=%s config=%s',
        nodeId,
        patch.name ?? '(unchanged)',
        patch.config ? 'present' : '(unchanged)'
      );
      setNodes(current =>
        current.map(n =>
          n.id === nodeId
            ? {
                ...n,
                data: {
                  ...n.data,
                  ...(patch.name !== undefined ? { name: patch.name } : {}),
                  ...(patch.config !== undefined ? { config: patch.config } : {}),
                },
              }
            : n
        )
      );
    },
    [setNodes]
  );

  // Close the drawer AND clear the selection, so re-clicking the same node
  // re-fires `onSelectionChange` and reopens it.
  const handleCloseConfig = useCallback(() => {
    log('closeConfig: deselecting all nodes');
    setConfigNodeId(null);
    setNodes(current =>
      current.some(n => n.selected) ? current.map(n => ({ ...n, selected: false })) : current
    );
  }, [setNodes]);

  const configNode = configNodeId ? (nodes.find(n => n.id === configNodeId) ?? null) : null;

  return (
    <div
      className="flow-canvas relative h-full w-full"
      data-testid="flow-canvas"
      data-editable="true"
      onDrop={handleDrop}
      onDragOver={handleDragOver}>
      <NodePalette onAdd={handlePaletteAdd} />

      <div className="pointer-events-none absolute right-3 top-3 z-10 flex items-center gap-2">
        {dirty && (
          <span
            className="pointer-events-auto rounded-full bg-amber-100 px-2 py-0.5 text-[11px] font-medium text-amber-700 dark:bg-amber-500/15 dark:text-amber-300"
            data-testid="flow-editor-dirty">
            {t('flows.editor.unsaved')}
          </span>
        )}
        <Button
          type="button"
          variant="secondary"
          tone="danger"
          size="xs"
          className="pointer-events-auto"
          data-testid="flow-editor-delete"
          disabled={selectionCount === 0}
          onClick={handleDeleteSelected}>
          {t('flows.editor.deleteSelected')}
        </Button>
        <Button
          type="button"
          variant="secondary"
          size="xs"
          className="pointer-events-auto"
          data-testid="flow-editor-validate"
          disabled={validating}
          onClick={handleValidate}>
          {validating ? t('flows.editor.validating') : t('flows.editor.validate')}
        </Button>
        <Button
          type="button"
          variant="tertiary"
          size="xs"
          className="pointer-events-auto"
          data-testid="flow-editor-discard"
          disabled={!dirty || saving}
          onClick={handleDiscard}>
          {t('flows.editor.discard')}
        </Button>
        {onSave && (
          <Button
            type="button"
            variant="primary"
            size="xs"
            className="pointer-events-auto"
            data-testid="flow-editor-save"
            title={hasErrors ? t('flows.editor.saveBlocked') : undefined}
            disabled={!dirty || hasErrors || saving || saveDisabled}
            onClick={handleSave}>
            {saving ? t('flows.editor.saving') : t('flows.editor.save')}
          </Button>
        )}
      </div>

      <div className="pointer-events-none absolute inset-x-3 bottom-3 z-10 flex justify-center">
        <div className="pointer-events-auto w-full max-w-md">
          <FlowValidationBanner validation={validation} saveError={saveError} />
        </div>
      </div>

      <ReactFlow
        nodes={displayNodes}
        edges={edges}
        nodeTypes={NODE_TYPES}
        onInit={instance => {
          rfRef.current = instance;
        }}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onConnect={onConnect}
        isValidConnection={isValidConnection}
        onSelectionChange={onSelectionChange}
        deleteKeyCode={DELETE_KEYS}
        nodesDraggable
        nodesConnectable
        elementsSelectable
        fitView
        panOnScroll
        zoomOnScroll>
        <Background variant={BackgroundVariant.Dots} gap={16} size={1} />
        <MiniMap pannable zoomable />
        <Controls showInteractive={false} />
      </ReactFlow>

      <NodeConfigDrawer
        node={configNode}
        onClose={handleCloseConfig}
        onChange={updateNode}
        connections={connections}
      />
    </div>
  );
}

export default memo(EditableFlowCanvas);
