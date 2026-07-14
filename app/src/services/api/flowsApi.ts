/**
 * Frontend client for the durable `openhuman.flows_*` run surface (issue B2 /
 * B3 / B4 / B5b). Wraps the subset of controllers the B3a approval card, the
 * B3b run inspector, the B4 agent-proposal card, and the B5b Workflow Canvas
 * need:
 *   - `flows_create`    — persist a new flow (B4 — only ever called from the
 *     user's own "Save & enable" click on `WorkflowProposalCard`; the agent's
 *     `propose_workflow` tool only validates and never reaches this RPC)
 *   - `flows_resume`    — resume a `pending_approval` run past its checkpoint
 *   - `flows_list_runs` — recent runs for a flow, newest first (B3b)
 *   - `flows_get_run`   — a single run record by id (B3b)
 *   - `flows_get`       — a single flow by id, graph included (B5b.1 canvas)
 *
 * Wire shape note: every `src/openhuman/flows/ops.rs` handler returns its
 * value via `RpcOutcome::single_log(value, "...")`, which
 * `into_cli_compatible_json` ALWAYS wraps as `{ result: value, logs: [...] }`
 * (see `src/rpc/mod.rs`) because a log message is always attached. `callCoreRpc`
 * already unwraps the outer JSON-RPC envelope, so the value this client
 * receives is that `{ result, logs }` object — {@link unwrapCliEnvelope} peels
 * it back to the actual payload. This mirrors the private helper of the same
 * name in `channelConnectionsApi.ts`.
 *
 * `flows_resume`'s payload is NOT a `FlowRun` row — it's the raw tinyflows
 * resume outcome (`{ output, pending_approvals, thread_id }`). The persisted
 * `FlowRun` record (with `status`/`steps`/etc.) only comes back from
 * `flows_list_runs` / `flows_get_run`. The approval card only needs to know
 * the resume call succeeded, so it doesn't need the full row.
 */
import debug from 'debug';

import type { WorkflowGraph } from '../../lib/flows/types';
import type { WorkflowProposal } from '../../store/chatRuntimeSlice';
import { callCoreRpc } from '../coreRpcClient';

const log = debug('flowsApi');

/**
 * `openhuman.flows_resume` and `openhuman.flows_run` both drive the tinyflows
 * engine and can run up to ~600s server-side (`FLOW_RUN_TIMEOUT_SECS` in
 * `src/openhuman/flows/ops.rs`). Give the client a slightly larger budget than
 * the default 30s so a slow run/resume doesn't fail client-side while the
 * engine is still running.
 */
const FLOW_RESUME_TIMEOUT_MS = 610_000;

// ---------------------------------------------------------------------------
// Wire types — mirror `src/openhuman/flows/types.rs`. No rename_all attribute
// on the Rust structs, so field names are snake_case on the wire as-is.
// ---------------------------------------------------------------------------

/** Lifecycle status of a durable flow run. */
export type FlowRunStatus =
  | 'running'
  | 'completed'
  | 'completed_with_warnings'
  | 'pending_approval'
  | 'failed'
  | 'cancelled';

/** One reconstructed step of a persisted `FlowRun` (`src/openhuman/flows/types.rs::FlowRunStep`). */
export interface FlowRunStep {
  node_id: string;
  output: unknown;
  /** Output port the node routed on, if any (branching/switch nodes). Omitted when absent. */
  port?: string;
  /**
   * Live step outcome as observed by `FlowRunObserver::on_step_finish` — `undefined`
   * for a step reconstructed post-hoc (e.g. the trigger node) rather than
   * observed live, not "unknown/neutral" in the UI sense.
   */
  status?: 'success' | 'error';
  /** Wall-clock duration of this step, if the observer recorded one. */
  duration_ms?: number;
  /**
   * Config `=`-expressions that resolved to `null` while running this step
   * (`location` is the config path, e.g. `args.to`). Empty/absent when clean.
   */
  diagnostics?: Array<{ location: string; expression: string }>;
}

/** A persisted flow run record (`src/openhuman/flows/types.rs::FlowRun`). */
export interface FlowRun {
  /** Same value as `thread_id` (the tinyflows checkpointer key). */
  id: string;
  flow_id: string;
  thread_id: string;
  status: FlowRunStatus;
  started_at: string;
  finished_at?: string | null;
  steps: FlowRunStep[];
  /** Node ids paused awaiting approval when `status === 'pending_approval'`. */
  pending_approvals: string[];
  error?: string | null;
}

/**
 * Raw resume outcome returned by `openhuman.flows_resume` — the immediate
 * tinyflows engine result, not the persisted `FlowRun` row. Call
 * {@link getFlowRun} afterwards (thread_id === run id) if the caller needs the
 * up-to-date persisted status.
 */
export interface FlowResumeResult {
  output: unknown;
  pending_approvals: string[];
  thread_id: string;
}

/**
 * A saved automation workflow (`src/openhuman/flows/types.rs::Flow`) — the
 * Workflows list page (B5a) row shape. `graph` is the raw tinyflows
 * `WorkflowGraph`; the list page doesn't need to interpret it, only the
 * canvas (B5b) does, so it's kept as `unknown` here.
 */
export interface Flow {
  /** Stable identifier (UUID) for this flow. */
  id: string;
  /** Human-readable name shown in the Workflows UI. */
  name: string;
  /** Whether this flow may currently be triggered/run. */
  enabled: boolean;
  /** The validated, migrated workflow graph — opaque to this client. */
  graph: unknown;
  /** RFC3339 creation timestamp. */
  created_at: string;
  /** RFC3339 last-update timestamp. */
  updated_at: string;
  /** RFC3339 timestamp of the most recent run, if any. */
  last_run_at: string | null;
  /** Outcome of the most recent run: `"completed"` | `"pending_approval"` | `"failed"`. */
  last_status: string | null;
  /** "Require approval for outbound actions" toggle (issue B2). */
  require_approval: boolean;
}

/**
 * Result of `openhuman.flows_validate` (`src/openhuman/flows/types.rs::FlowValidation`).
 * `valid === false` means the graph is structurally rejected and won't
 * persist/enable; `warnings` are advisory and orthogonal to validity (a valid
 * graph can still carry them). `errors` carries at most one message — the
 * first structural error tinyflows's validator reports — so it's a
 * graph-level signal, not a per-node list.
 */
export interface FlowValidation {
  valid: boolean;
  errors: string[];
  warnings: string[];
}

/**
 * Source format for {@link importFlow}. `native` is a tinyflows `WorkflowGraph`
 * JSON; `n8n` is an n8n workflow export (mapped best-effort host-side); `auto`
 * (the default) detects the shape.
 */
export type FlowImportFormat = 'native' | 'n8n' | 'auto';

/**
 * Result of `openhuman.flows_import` (`src/openhuman/flows/types.rs::FlowImport`).
 * The `graph` is the normalized, migrated + validated `WorkflowGraph` ready to
 * open on the canvas as an unsaved draft; `warnings` carries non-fatal import
 * notes (unmapped n8n node types, untranslated expressions, a synthesized or
 * demoted trigger). Import NEVER persists — the user Saves via the normal gate.
 */
export interface FlowImport {
  graph: unknown;
  warnings: string[];
}

/**
 * A secret-free credential reference for the node-config credential picker
 * (`src/openhuman/flows/types.rs::FlowConnection`). `connection_ref` is
 * `"composio:<toolkit>:<connection_id>"` (composio) or `"http_cred:<name>"`
 * (raw HTTP cred). `toolkit` is present only for composio; `scheme`
 * (`"bearer"|"basic"|"header"`) only for http.
 */
export interface FlowConnection {
  connection_ref: string;
  kind: 'composio' | 'http';
  display: string;
  toolkit?: string;
  scheme?: string;
}

/** Optional fields for {@link updateFlow}. Omitted fields are left untouched. */
export interface FlowUpdate {
  name?: string;
  graph?: unknown;
  requireApproval?: boolean;
}

/** Lifecycle status of a {@link FlowSuggestion} (`src/openhuman/flows/types.rs::SuggestionStatus`). */
export type SuggestionStatus = 'new' | 'dismissed' | 'built';

/**
 * A Flow Scout workflow suggestion (`src/openhuman/flows/types.rs::FlowSuggestion`)
 * — a *pitch*, not a graph. Rendered as a card in the Flows page "Suggested for
 * you" section. `build_prompt` is the natural-language brief handed to the
 * `workflow_builder` agent when the user clicks "Build this".
 */
export interface FlowSuggestion {
  id: string;
  title: string;
  one_liner: string;
  rationale: string;
  /** `schedule` | `app_event` | `manual` — omitted when the agent didn't set one. */
  trigger_hint?: string | null;
  steps_outline: string[];
  suggested_connections: string[];
  suggested_slugs: string[];
  build_prompt: string;
  confidence: number;
  status: SuggestionStatus;
  created_at: string;
  source_run_id?: string | null;
}

// ---------------------------------------------------------------------------
// CLI-compatible envelope unwrapping.
// ---------------------------------------------------------------------------

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null;
  }
  return value as Record<string, unknown>;
}

/**
 * Every `flows_*` handler goes through `RpcOutcome::single_log`, so the value
 * `callCoreRpc` resolves is always `{ result: <payload>, logs: string[] }`.
 * Peel that back to `<payload>`. Falls through unchanged if the shape doesn't
 * match (defensive — keeps this client working if a future handler switches
 * to a log-less `RpcOutcome::new` and stops wrapping).
 */
function unwrapCliEnvelope<T>(payload: unknown): T {
  const record = asRecord(payload);
  if (record && 'result' in record && 'logs' in record && Array.isArray(record.logs)) {
    return record.result as T;
  }
  return payload as T;
}

// ---------------------------------------------------------------------------
// RPC client.
// ---------------------------------------------------------------------------

/**
 * Create (and, by default, enable) a new saved flow via `openhuman.flows_create`
 * (issue B4). This is the ONLY path that persists a flow — the agent's
 * `propose_workflow` tool (`src/openhuman/flows/tools.rs`) only validates a
 * candidate graph and returns a summary; `WorkflowProposalCard`'s "Save &
 * enable" button is what calls this function, directly from the client, on
 * the user's explicit action. `requireApproval` defaults server-side to
 * `false` when omitted, but the B4 proposal flow always passes it explicitly
 * (defaulting to `true` on the Rust tool side) so a saved agent-proposed flow
 * starts with its outbound-action approval gate on.
 *
 * B29 (save/enable safety) Rule 1: when `graph`'s trigger fires without a
 * human in the loop (`schedule` / `app_event` / `webhook`), the server
 * ALWAYS persists the flow `enabled: false`, regardless of what the caller
 * intended — no creation path may silently hand back an armed, unattended
 * automation. The returned {@link Flow}'s `enabled` field reflects this, so
 * every caller MUST check it: if the caller represents an explicit
 * user-arming action (e.g. `WorkflowProposalCard`'s "Save & enable" click),
 * follow up with {@link setFlowEnabled} to actually arm it — that is a
 * legitimate explicit enable, not the silent copilot auto-arm Rule 1 guards
 * against. A caller with no such explicit intent (e.g. a background/copilot
 * save) should leave it disabled and let the user arm it later.
 */
export async function createFlow(
  name: string,
  graph: unknown,
  requireApproval?: boolean
): Promise<Flow> {
  log('createFlow: request name=%s requireApproval=%s', name, requireApproval ?? 'default');
  const response = await callCoreRpc<unknown>({
    method: 'openhuman.flows_create',
    params:
      requireApproval === undefined
        ? { name, graph }
        : { name, graph, require_approval: requireApproval },
  });
  const flow = unwrapCliEnvelope<Flow>(response);
  log('createFlow: response id=%s name=%s enabled=%s', flow.id, flow.name, flow.enabled);
  return flow;
}

/**
 * Resume a `pending_approval` flow run past its checkpoint via
 * `openhuman.flows_resume`. `approvals` should name the node ids from the
 * triggering notification's `node_ids` payload — the Rust side rejects the
 * call outright unless at least one named id matches a currently-pending gate.
 */
export async function resumeFlow(
  id: string,
  threadId: string,
  approvals: string[]
): Promise<FlowResumeResult> {
  log('resumeFlow: request id=%s threadId=%s approvals=%o', id, threadId, approvals);
  const response = await callCoreRpc<unknown>({
    method: 'openhuman.flows_resume',
    params: { id, thread_id: threadId, approvals },
    timeoutMs: FLOW_RESUME_TIMEOUT_MS,
  });
  const result = unwrapCliEnvelope<FlowResumeResult>(response);
  log(
    'resumeFlow: response threadId=%s pendingApprovals=%d',
    result.thread_id,
    result.pending_approvals?.length ?? 0
  );
  return result;
}

/**
 * List recent runs for a flow, newest first, via `openhuman.flows_list_runs`.
 * `limit` defaults to 20 server-side. Not used by the B3a approval card —
 * exported now for the B3b run-history inspector.
 */
export async function listFlowRuns(flowId: string, limit?: number): Promise<FlowRun[]> {
  log('listFlowRuns: request flowId=%s limit=%s', flowId, limit ?? 'default');
  const response = await callCoreRpc<unknown>({
    method: 'openhuman.flows_list_runs',
    params: limit === undefined ? { id: flowId } : { id: flowId, limit },
  });
  const runs = unwrapCliEnvelope<FlowRun[]>(response);
  log('listFlowRuns: response count=%d', runs.length);
  return runs;
}

/**
 * Load a single flow run record by id (== thread_id) via
 * `openhuman.flows_get_run`. Not used by the B3a approval card — exported now
 * for the B3b run-history inspector.
 */
export async function getFlowRun(runId: string): Promise<FlowRun> {
  log('getFlowRun: request runId=%s', runId);
  const response = await callCoreRpc<unknown>({
    method: 'openhuman.flows_get_run',
    params: { run_id: runId },
  });
  const run = unwrapCliEnvelope<FlowRun>(response);
  log('getFlowRun: response id=%s status=%s', run.id, run.status);
  return run;
}

/**
 * List all saved flows via `openhuman.flows_list` (the Workflows list page,
 * B5a). No params. Unlike the run-surface calls above, the payload IS the
 * `Flow[]` array directly — there is no outer `{ flows: [...] }` wrapper (see
 * `src/openhuman/flows/ops.rs::flows_list`, which returns `Vec<Flow>`
 * straight through `RpcOutcome::single_log`).
 */
export async function listFlows(): Promise<Flow[]> {
  log('listFlows: request');
  const response = await callCoreRpc<unknown>({ method: 'openhuman.flows_list', params: {} });
  const flows = unwrapCliEnvelope<Flow[]>(response);
  log('listFlows: response count=%d', flows.length);
  return flows;
}

/**
 * Enable or disable a saved flow via `openhuman.flows_set_enabled`. Returns
 * the updated `Flow` row directly (same no-wrapper shape as `flows_list`'s
 * elements).
 */
export async function setFlowEnabled(id: string, enabled: boolean): Promise<Flow> {
  log('setFlowEnabled: request id=%s enabled=%s', id, enabled);
  const response = await callCoreRpc<unknown>({
    method: 'openhuman.flows_set_enabled',
    params: { id, enabled },
  });
  const flow = unwrapCliEnvelope<Flow>(response);
  log('setFlowEnabled: response id=%s enabled=%s', flow.id, flow.enabled);
  return flow;
}

/**
 * Load a single saved flow by id via `openhuman.flows_get` (the Workflow
 * Canvas, B5b.1). Returns the `Flow` directly (same no-wrapper shape as
 * `flows_list`'s elements and `flows_set_enabled` — see
 * `src/openhuman/flows/schemas.rs::handle_get`, which delegates straight to
 * `ops::flows_get` through `RpcOutcome::single_log`).
 */
export async function getFlow(id: string): Promise<Flow> {
  log('getFlow: request id=%s', id);
  const response = await callCoreRpc<unknown>({ method: 'openhuman.flows_get', params: { id } });
  const flow = unwrapCliEnvelope<Flow>(response);
  log('getFlow: response id=%s name=%s', flow.id, flow.name);
  return flow;
}

/**
 * Run a saved flow to completion (or until it pauses on a human-approval
 * gate) via `openhuman.flows_run`. This is the call that actually drives the
 * tinyflows engine, so it shares `flows_resume`'s ~600s server-side budget
 * (see {@link FLOW_RESUME_TIMEOUT_MS}). The Workflows list page's Run button
 * uses this fire-and-forget: it awaits the call just long enough to know the
 * run kicked off, shows a toast, and refetches `listFlows()` to pick up the
 * refreshed `last_run_at`/`last_status`.
 */
export async function runFlow(id: string, input?: unknown): Promise<FlowResumeResult> {
  log('runFlow: request id=%s', id);
  const response = await callCoreRpc<unknown>({
    method: 'openhuman.flows_run',
    params: { id, input: input ?? null },
    timeoutMs: FLOW_RESUME_TIMEOUT_MS,
  });
  const result = unwrapCliEnvelope<FlowResumeResult>(response);
  log(
    'runFlow: response threadId=%s pendingApprovals=%d',
    result.thread_id,
    result.pending_approvals?.length ?? 0
  );
  return result;
}

/**
 * Permanently delete a saved flow via `openhuman.flows_delete`. The server
 * unbinds any live trigger (schedule cron job / app-event binding) before
 * removing the row, so deleting an enabled flow also stops it firing. Returns
 * the removed id (the payload is `{ id, removed: true }`); callers typically
 * just refetch the list.
 */
export async function deleteFlow(id: string): Promise<string> {
  log('deleteFlow: request id=%s', id);
  const response = await callCoreRpc<unknown>({ method: 'openhuman.flows_delete', params: { id } });
  const payload = unwrapCliEnvelope<{ id: string; removed: boolean }>(response);
  log('deleteFlow: response id=%s removed=%s', payload.id, payload.removed);
  return payload.id;
}

/**
 * Duplicate a saved flow via `openhuman.flows_duplicate`. The copy is created
 * DISABLED and unbound (no live trigger), with a derived name, so duplicating an
 * enabled flow never silently starts a second live schedule. Returns the new
 * `Flow` row.
 */
export async function duplicateFlow(id: string): Promise<Flow> {
  log('duplicateFlow: request id=%s', id);
  const response = await callCoreRpc<unknown>({
    method: 'openhuman.flows_duplicate',
    params: { id },
  });
  const flow = unwrapCliEnvelope<Flow>(response);
  log('duplicateFlow: response newId=%s name=%s', flow.id, flow.name);
  return flow;
}

/**
 * Update a saved flow's name and/or graph via `openhuman.flows_update` (the
 * Workflow Canvas Save path, B5b.2 / Phase 3d). The server re-validates the
 * graph before persisting and rejects a structurally-invalid one, so callers
 * should {@link validateFlow} first to surface errors pre-save. Omitted fields
 * are left untouched; returns the updated `Flow` row.
 */
export async function updateFlow(id: string, update: FlowUpdate): Promise<Flow> {
  log(
    'updateFlow: request id=%s name=%s graph=%s requireApproval=%s',
    id,
    update.name ?? '(unchanged)',
    update.graph === undefined ? '(unchanged)' : 'present',
    update.requireApproval ?? 'unchanged'
  );
  const params: Record<string, unknown> = { id };
  if (update.name !== undefined) params.name = update.name;
  if (update.graph !== undefined) params.graph = update.graph;
  if (update.requireApproval !== undefined) params.require_approval = update.requireApproval;
  const response = await callCoreRpc<unknown>({ method: 'openhuman.flows_update', params });
  const flow = unwrapCliEnvelope<Flow>(response);
  log('updateFlow: response id=%s name=%s', flow.id, flow.name);
  return flow;
}

/**
 * Validate a candidate `WorkflowGraph` via `openhuman.flows_validate`. Pure and
 * cheap server-side (no config load), so it's safe to call on a debounce while
 * editing. Returns {@link FlowValidation} — check `valid` to gate Save, and
 * surface `warnings` separately (they never block).
 */
export async function validateFlow(graph: unknown): Promise<FlowValidation> {
  log('validateFlow: request');
  const response = await callCoreRpc<unknown>({
    method: 'openhuman.flows_validate',
    params: { graph },
  });
  const validation = unwrapCliEnvelope<FlowValidation>(response);
  log(
    'validateFlow: response valid=%s errors=%d warnings=%d',
    validation.valid,
    validation.errors.length,
    validation.warnings.length
  );
  return validation;
}

/**
 * List the secret-free credential references (composio + http) available to a
 * node's config credential picker via `openhuman.flows_list_connections`. No
 * params; returns the `FlowConnection[]` directly (same no-wrapper shape as
 * `flows_list`).
 */
export async function listFlowConnections(): Promise<FlowConnection[]> {
  log('listFlowConnections: request');
  const response = await callCoreRpc<unknown>({
    method: 'openhuman.flows_list_connections',
    params: {},
  });
  const connections = unwrapCliEnvelope<FlowConnection[]>(response);
  log('listFlowConnections: response count=%d', connections.length);
  return connections;
}

/**
 * Import a workflow definition (native tinyflows JSON or an n8n export) via
 * `openhuman.flows_import`. The server migrates + validates it host-side and
 * returns the normalized graph plus non-fatal warnings WITHOUT persisting — the
 * caller opens the result on the canvas as a draft and Saves via the existing
 * `flows_create` gate. Rejects (throws) when the definition is structurally
 * invalid or unparseable server-side, so the UI can surface a load error
 * instead of opening a broken canvas.
 */
export async function importFlow(
  graph: unknown,
  format: FlowImportFormat = 'auto'
): Promise<FlowImport> {
  log('importFlow: request format=%s', format);
  const response = await callCoreRpc<unknown>({
    method: 'openhuman.flows_import',
    params: { graph, format },
  });
  const result = unwrapCliEnvelope<FlowImport>(response);
  log('importFlow: response warnings=%d', result.warnings?.length ?? 0);
  return result;
}

/**
 * `openhuman.flows_discover` runs the read-only Flow Scout agent, which reasons
 * over the user's memory/threads/connections/flows and can take up to ~300s
 * server-side (`FLOW_DISCOVER_TIMEOUT_SECS` in `src/openhuman/flows/ops.rs`).
 * Give the client a matching budget so a slow discovery run doesn't time out
 * client-side while the agent is still thinking.
 */
const FLOW_DISCOVER_TIMEOUT_MS = 310_000;

/**
 * Run the Flow Scout discovery agent via `openhuman.flows_discover` and return
 * the active (new) suggestions it produced. Read-only server-side — it never
 * creates, enables, or runs a flow. Returns the `FlowSuggestion[]` directly
 * (same no-wrapper shape as `flows_list`).
 */
export async function discoverWorkflows(threadId?: string | null): Promise<FlowSuggestion[]> {
  log('discoverWorkflows: request thread=%s', threadId ?? '<none>');
  // When a caller passes a chat thread id, the server streams the Flow Scout
  // turn's text/tool events onto that thread (Phase B) so a shared chat pane can
  // render them live. The param name matches the `thread_id` convention in
  // `src/openhuman/flows/schemas.rs` (a per-turn `request_id` is minted
  // server-side when omitted). Omitting it keeps the headless behaviour.
  const params: Record<string, unknown> = {};
  if (threadId) params.thread_id = threadId;
  const response = await callCoreRpc<unknown>({
    method: 'openhuman.flows_discover',
    params,
    timeoutMs: FLOW_DISCOVER_TIMEOUT_MS,
  });
  const suggestions = unwrapCliEnvelope<FlowSuggestion[]>(response);
  log('discoverWorkflows: response count=%d', suggestions.length);
  return suggestions;
}

/**
 * List persisted workflow suggestions via `openhuman.flows_list_suggestions`.
 * `status` filters to one lifecycle state (`new` for the active cards); omit
 * for all. Returns the `FlowSuggestion[]` directly.
 */
export async function listSuggestions(status?: SuggestionStatus): Promise<FlowSuggestion[]> {
  log('listSuggestions: request status=%s', status ?? 'all');
  const response = await callCoreRpc<unknown>({
    method: 'openhuman.flows_list_suggestions',
    params: status === undefined ? {} : { status },
  });
  const suggestions = unwrapCliEnvelope<FlowSuggestion[]>(response);
  log('listSuggestions: response count=%d', suggestions.length);
  return suggestions;
}

// ─────────────────────────────────────────────────────────────────────────────
// flows_build — run the workflow_builder agent for one authoring turn.
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Which authoring turn to run (mirrors the Rust `BuildMode`). The server renders
 * the agent's natural-language brief from this — the frontend no longer crafts
 * delegate prompts.
 */
export type BuilderTurnMode = 'create' | 'revise' | 'repair' | 'build';

/** A structured workflow-builder turn request. */
export interface BuilderTurnRequest {
  /** Which kind of turn to run. */
  mode: BuilderTurnMode;
  /** The user's ask: description (create/build) or change instruction (revise). */
  instruction: string;
  /** The current draft graph, injected as context for revise/repair/build. */
  graph?: WorkflowGraph | null;
  /** Saved flow id (required for `build`; optional elsewhere for run-to-test). */
  flowId?: string | null;
  /** Failed run id (== thread id) for `repair`. */
  runId?: string | null;
  /** Run-level error message for `repair`, if known. */
  error?: string | null;
  /** Node ids implicated in the failure, for `repair`. */
  failingNodeIds?: string[];
}

/** The result of one builder turn. */
export interface BuilderTurnResult {
  /** The proposal the agent produced (mapped to the store shape), or null. */
  proposal: WorkflowProposal | null;
  /** The agent's final assistant text (rendered as its chat turn). */
  assistantText: string;
  /** A run error, if the turn failed but a prior proposal was still captured. */
  error: string | null;
  /**
   * `true` when the turn paused because it hit the agent's tool-call budget
   * (`max_tool_iterations`) with no proposal yet — as opposed to the agent
   * voluntarily asking a clarifying question or finishing. `assistantText` is
   * a "Done so far / Next steps" checkpoint in this case; the UI should offer
   * a "Continue building" action rather than rendering it as a normal reply.
   */
  capped: boolean;
}

/**
 * The `workflow_builder` agent can take up to ~600s server-side
 * (`FLOW_BUILD_TIMEOUT_SECS` in `src/openhuman/flows/ops.rs`); match it so a slow
 * authoring turn doesn't time out client-side while the agent is still working.
 */
const FLOW_BUILD_TIMEOUT_MS = 610_000;

/**
 * Map a raw `{ type: 'workflow_proposal', … }` payload (from the agent's
 * propose/revise/save tool) to the store {@link WorkflowProposal} shape. Kept in
 * lockstep with `parseWorkflowProposal` in `ChatRuntimeProvider` (the streamed
 * path); returns null if the payload isn't a valid proposal.
 */
export function mapWorkflowProposal(payload: unknown): WorkflowProposal | null {
  if (!payload || typeof payload !== 'object') return null;
  const obj = payload as Record<string, unknown>;
  if (obj.type !== 'workflow_proposal') return null;
  if (typeof obj.name !== 'string' || obj.graph == null) return null;

  const summary = (obj.summary ?? {}) as Record<string, unknown>;
  const rawSteps = Array.isArray(summary.steps) ? summary.steps : [];
  const steps = rawSteps
    .filter((s): s is Record<string, unknown> => !!s && typeof s === 'object')
    .map(s => ({
      kind: typeof s.kind === 'string' ? s.kind : 'unknown',
      name: typeof s.name === 'string' ? s.name : '',
      config_hint: typeof s.config_hint === 'string' ? s.config_hint : undefined,
    }));

  return {
    name: obj.name,
    graph: obj.graph,
    // The Rust tool defaults `require_approval` to true when omitted, so treat
    // anything other than an explicit false as true — in lockstep with the server.
    requireApproval: obj.require_approval !== false,
    summary: { trigger: typeof summary.trigger === 'string' ? summary.trigger : '', steps },
  };
}

/**
 * Run one `workflow_builder` authoring turn via `openhuman.flows_build`. The
 * server renders the agent's brief from `request`, runs the agent to completion,
 * and returns its proposal + final assistant text. This is the backend-agent
 * path that replaces the frontend's old "craft a delegate prompt and route it
 * through the chat orchestrator" approach.
 */
export async function buildWorkflow(
  request: BuilderTurnRequest,
  threadId?: string | null
): Promise<BuilderTurnResult> {
  log(
    'buildWorkflow: request mode=%s flowId=%s thread=%s',
    request.mode,
    request.flowId ?? '<none>',
    threadId ?? '<none>'
  );
  const params: Record<string, unknown> = {
    mode: request.mode,
    instruction: request.instruction,
    graph: request.graph ?? null,
    flow_id: request.flowId ?? null,
    run_id: request.runId ?? null,
    error: request.error ?? null,
    failing_node_ids: request.failingNodeIds ?? [],
  };
  // When the copilot passes its dedicated chat thread id, the server streams the
  // builder turn's text/thinking/tool events onto that thread (Phase B) so the
  // shared chat pane renders them live and `ChatRuntimeProvider` appends the
  // final assistant message on `chat_done`. Param name matches the `thread_id`
  // convention in `src/openhuman/flows/schemas.rs`; a per-turn `request_id` is
  // minted server-side when omitted. Omitting it keeps the headless behaviour.
  if (threadId) params.thread_id = threadId;
  const response = await callCoreRpc<unknown>({
    method: 'openhuman.flows_build',
    params,
    timeoutMs: FLOW_BUILD_TIMEOUT_MS,
  });
  const result = unwrapCliEnvelope<{
    proposal: unknown;
    assistant_text: string;
    error: string | null;
    capped?: boolean;
  }>(response);
  log(
    'buildWorkflow: response hasProposal=%s capped=%s',
    result.proposal != null,
    result.capped ?? false
  );
  return {
    proposal: mapWorkflowProposal(result.proposal),
    assistantText: result.assistant_text ?? '',
    error: result.error ?? null,
    capped: result.capped ?? false,
  };
}

/**
 * Dismiss a suggestion via `openhuman.flows_dismiss_suggestion` (the user
 * rejected the card). The row is kept server-side so a later discovery run
 * dedupes against it and won't re-surface the idea.
 */
export async function dismissSuggestion(id: string): Promise<boolean> {
  log('dismissSuggestion: request id=%s', id);
  const response = await callCoreRpc<unknown>({
    method: 'openhuman.flows_dismiss_suggestion',
    params: { id },
  });
  const result = unwrapCliEnvelope<{ id: string; dismissed: boolean }>(response);
  log('dismissSuggestion: response dismissed=%s', result.dismissed);
  return result.dismissed;
}

/**
 * Mark a suggestion as built via `openhuman.flows_mark_suggestion_built` —
 * called after the user saves a flow authored from it, so it drops out of the
 * active cards.
 */
export async function markSuggestionBuilt(id: string): Promise<boolean> {
  log('markSuggestionBuilt: request id=%s', id);
  const response = await callCoreRpc<unknown>({
    method: 'openhuman.flows_mark_suggestion_built',
    params: { id },
  });
  const result = unwrapCliEnvelope<{ id: string; built: boolean }>(response);
  log('markSuggestionBuilt: response built=%s', result.built);
  return result.built;
}

export const flowsApi = {
  createFlow,
  importFlow,
  discoverWorkflows,
  listSuggestions,
  dismissSuggestion,
  markSuggestionBuilt,
  resumeFlow,
  listFlowRuns,
  getFlowRun,
  getFlow,
  listFlows,
  setFlowEnabled,
  runFlow,
  updateFlow,
  deleteFlow,
  duplicateFlow,
  validateFlow,
  listFlowConnections,
};

export default flowsApi;
