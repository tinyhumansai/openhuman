import { callCoreRpc } from '../coreRpcClient';

// ---------------------------------------------------------------------------
// Approval audit / history read client.
//
// Surfaces the read paths added in PR #2335 (`approval_list_recent_decisions`)
// and the live `approval_list_pending` queue. Both are exposed by the core's
// approval gate through the controller registry; this client only READS them —
// decisions still flow through `openhuman.approval_decide` (ApprovalRequestCard).
//
// Wire-shape note: both RPCs return an `RpcOutcome` with a single diagnostic
// log line when the gate is installed, so the JSON-RPC `result` is the
// CLI-compatible envelope `{ result: [...rows], logs: [...] }`. When the gate
// is NOT installed the core returns a bare `[]`. `unwrapRows` normalizes both.
// ---------------------------------------------------------------------------

/**
 * User's decision on a pending approval (mirrors Rust `ApprovalDecision`).
 * `approve_always_for_flow` (flow-approval surface, see {@link ApprovalSourceContext})
 * additionally allowlists the gate for every future run of the same flow, the
 * flow-scoped analogue of `approve_always_for_tool`'s session-scoped allowlist.
 */
export type ApprovalDecision =
  | 'approve_once'
  | 'approve_always_for_tool'
  | 'approve_always_for_flow'
  | 'deny';

/**
 * Origin hint attached to a pending approval that was raised from inside a
 * `tinyflows` run rather than an interactive chat turn. Lets the flow-run
 * inspector (and any other flow-aware surface) filter the shared
 * `approval_list_pending` queue down to just the gates for one run without a
 * dedicated endpoint. Absent for chat-originated approvals.
 */
export interface ApprovalSourceContext {
  kind: 'flow';
  flow_id: string;
  run_id: string;
  node_id?: string;
}

/** A pending approval awaiting a decision (mirrors Rust `PendingApproval`). */
export interface PendingApproval {
  request_id: string;
  tool_name: string;
  /** Short human-readable summary, scrubbed of PII / chat content. */
  action_summary: string;
  /** Redacted JSON arguments — counts/shape only, no raw message bodies. */
  args_redacted: unknown;
  session_id: string;
  /** RFC3339 timestamp. */
  created_at: string;
  /** RFC3339 timestamp, or null when the request does not expire. */
  expires_at: string | null;
  /** Present when this gate was raised from a flow run — see {@link ApprovalSourceContext}. */
  source_context?: ApprovalSourceContext;
}

/** A decided approval audit row (mirrors Rust `ApprovalAuditEntry`). */
export interface ApprovalAuditEntry {
  request_id: string;
  tool_name: string;
  action_summary: string;
  args_redacted: unknown;
  session_id: string;
  created_at: string;
  expires_at: string | null;
  /** RFC3339 timestamp the decision was recorded. */
  decided_at: string;
  decision: ApprovalDecision;
}

/**
 * Normalize the two possible wire shapes into a plain row array:
 * - gate installed → `{ result: T[], logs: string[] }`
 * - gate absent    → bare `T[]`
 * Anything else (unexpected) collapses to an empty array rather than throwing,
 * so a degraded core can never blank the whole settings screen.
 */
export const unwrapRows = <T>(raw: unknown): T[] => {
  if (Array.isArray(raw)) return raw as T[];
  if (raw && typeof raw === 'object' && Array.isArray((raw as { result?: unknown }).result)) {
    return (raw as { result: T[] }).result;
  }
  return [];
};

/** Default page size matching the core's `list_recent_decisions` default. */
export const DEFAULT_APPROVAL_HISTORY_LIMIT = 50;

/**
 * Fetch recently decided approval rows for the audit/history surface.
 * `limit` is clamped core-side; omit to use the core default (50).
 */
export const fetchRecentApprovalDecisions = async (
  limit?: number
): Promise<ApprovalAuditEntry[]> => {
  const raw = await callCoreRpc<unknown>({
    method: 'openhuman.approval_list_recent_decisions',
    params: limit === undefined ? {} : { limit },
  });
  return unwrapRows<ApprovalAuditEntry>(raw);
};

/** Fetch the live queue of pending (undecided) approvals. */
export const fetchPendingApprovals = async (): Promise<PendingApproval[]> => {
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.approval_list_pending' });
  return unwrapRows<PendingApproval>(raw);
};

/**
 * Record a decision on a pending approval. Shared by every approval surface
 * (chat `ApprovalRequestCard`, the flow-run inspector, the flow chat banner,
 * and the notification-center gate card) — they all park on the same
 * `ApprovalGate` and resolve through this one RPC.
 */
export const decideApproval = async (
  requestId: string,
  decision: ApprovalDecision
): Promise<void> => {
  await callCoreRpc({
    method: 'openhuman.approval_decide',
    params: { request_id: requestId, decision },
  });
};

/**
 * Snapshot of the host-aware approval-gate boot decision. Mirrors the Rust
 * `ApprovalGateBootState` struct in `src/openhuman/approval/gate.rs`.
 *
 * - `installed` — gate was installed at boot and `external_effect` tool calls
 *   will be intercepted.
 * - `disabledByEnv` — operator set `OPENHUMAN_APPROVAL_GATE=0` AND the host
 *   honored it (CLI / Docker). Gate is OFF; the UI shows the persistent red
 *   banner.
 * - `overrideIgnored` — operator set `OPENHUMAN_APPROVAL_GATE=0` under the
 *   Tauri desktop shell, which always ignores the override. The UI shows a
 *   one-shot yellow info banner so the user knows the attempt was rejected.
 * - `host` — `"tauri-shell"` / `"cli"` / `"docker"` / `"unknown"` (boot
 *   state was never recorded — older tests / direct gate spawn paths).
 */
export interface ApprovalGateBootState {
  installed: boolean;
  disabledByEnv: boolean;
  overrideIgnored: boolean;
  host: string;
}

const unwrapValue = <T>(raw: unknown): T => {
  if (raw && typeof raw === 'object' && 'result' in (raw as Record<string, unknown>)) {
    return (raw as { result: T }).result;
  }
  return raw as T;
};

/**
 * Fetch the boot-time approval-gate state for the security banner. Returns a
 * benign "no banner needed" fallback when the call fails, so a degraded core
 * can never blank the whole app shell.
 */
export const fetchApprovalGateState = async (): Promise<ApprovalGateBootState> => {
  try {
    const raw = await callCoreRpc<unknown>({ method: 'openhuman.approval_get_gate_state' });
    return unwrapValue<ApprovalGateBootState>(raw);
  } catch {
    return { installed: true, disabledByEnv: false, overrideIgnored: false, host: 'unknown' };
  }
};
