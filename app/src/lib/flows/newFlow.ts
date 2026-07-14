/**
 * Helpers for authoring a *new* flow from the Phase 4a chooser. Pure and
 * dependency-free so the blank-graph shape is unit-testable and the create
 * path (`flows_create`) is the only thing the UI has to wire up.
 */
import type { WorkflowGraph } from './types';

/** Stable node id of the starter trigger a "start from scratch" flow ships with. */
export const BLANK_TRIGGER_NODE_ID = 'trigger';

/**
 * A minimal, structurally-valid `WorkflowGraph` for "start from scratch": a
 * single `manual` trigger node and no edges. Passes the same
 * `openhuman.flows_validate` rules the templates do (exactly one trigger,
 * unique ids, no dangling edges), so `flows_create` accepts it directly.
 *
 * `name` is used for both the flow name (passed separately to `flows_create`)
 * and the graph's own `name`; `triggerName` is the human label shown on the
 * starter node in the canvas.
 */
/** Longest flow name we derive from a free-text prompt before truncating. */
export const MAX_DERIVED_NAME_LENGTH = 60;

/**
 * Derive a human-readable flow name from a free-text workflow description
 * (the prompt-bar's instant-create path names the flow before the builder
 * agent has proposed anything). First line only, whitespace-collapsed, and
 * truncated with an ellipsis past {@link MAX_DERIVED_NAME_LENGTH}. Falls back
 * to `fallback` (the localized "New workflow") when the description is blank.
 */
export function deriveWorkflowName(description: string, fallback: string): string {
  const firstLine = (description.split('\n', 1)[0] ?? '').replace(/\s+/g, ' ').trim();
  if (!firstLine) return fallback;
  if (firstLine.length <= MAX_DERIVED_NAME_LENGTH) return firstLine;
  return `${firstLine.slice(0, MAX_DERIVED_NAME_LENGTH - 1).trimEnd()}…`;
}

export function createBlankWorkflowGraph(name: string, triggerName: string): WorkflowGraph {
  return {
    schema_version: 1,
    name,
    nodes: [
      {
        id: BLANK_TRIGGER_NODE_ID,
        kind: 'trigger',
        name: triggerName,
        config: { trigger_kind: 'manual' },
        ports: [],
        position: { x: 0, y: 0 },
      },
    ],
    edges: [],
  };
}
