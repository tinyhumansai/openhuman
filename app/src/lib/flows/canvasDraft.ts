/**
 * Canvas draft handoff (Phase 4e) — carries an *unsaved* candidate
 * `WorkflowGraph` from the chat `WorkflowProposalCard` "Open in canvas" action
 * into the editable Workflow Canvas so the user can review/edit it BEFORE the
 * single persistence gate.
 *
 * Critical invariant: opening a draft NEVER persists or enables a flow. The
 * draft rides in the router's `location.state` (ephemeral, dropped on reload —
 * exactly what an unsaved draft should be) rather than any store or RPC. The
 * canvas's own Save button remains the one and only persistence gate; for a
 * draft it calls `flows_create` (see `FlowCanvasPage`), never on open.
 */
import type { WorkflowGraph } from './types';

/**
 * Dedicated route for an unsaved draft canvas. Placed BEFORE `/flows/:id` so it
 * matches first — otherwise `:id` would capture `"draft"` and try to
 * `flows_get('draft')`.
 */
export const FLOW_CANVAS_DRAFT_ROUTE = '/flows/draft';

/**
 * The shape stashed in `location.state` when navigating to
 * {@link FLOW_CANVAS_DRAFT_ROUTE}. Mirrors the fields `flows_create` needs so
 * the canvas's Save can persist the reviewed draft as-is.
 */
export interface FlowCanvasDraftState {
  /** Proposed flow name — seeds the canvas title and the eventual `flows_create`. */
  name: string;
  /** The candidate graph to open as an editable, unsaved draft. */
  graph: WorkflowGraph;
  /** "Require approval for outbound actions" toggle to carry into `flows_create`. */
  requireApproval: boolean;
  /**
   * Non-fatal import warnings (Phase 4d) — surfaced as toasts over the draft
   * canvas when a graph was imported via `flows_import` (unmapped n8n node
   * types, untranslated expressions, a synthesized/demoted trigger). Absent for
   * a chat-proposal draft (which carries no import notes).
   */
  importWarnings?: string[];
}

/** Narrow an opaque `location.state` to a {@link FlowCanvasDraftState}. */
export function asFlowCanvasDraftState(state: unknown): FlowCanvasDraftState | null {
  if (!state || typeof state !== 'object') return null;
  const record = state as Record<string, unknown>;
  const graph = record.graph;
  if (
    typeof record.name !== 'string' ||
    !graph ||
    typeof graph !== 'object' ||
    typeof record.requireApproval !== 'boolean'
  ) {
    return null;
  }
  const importWarnings = Array.isArray(record.importWarnings)
    ? record.importWarnings.filter((w): w is string => typeof w === 'string')
    : undefined;
  return {
    name: record.name,
    graph: graph as WorkflowGraph,
    requireApproval: record.requireApproval,
    ...(importWarnings ? { importWarnings } : {}),
  };
}
