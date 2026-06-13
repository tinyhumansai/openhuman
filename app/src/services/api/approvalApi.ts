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

/** User's decision on a pending approval (mirrors Rust `ApprovalDecision`). */
export type ApprovalDecision = 'approve_once' | 'approve_always_for_tool' | 'deny';

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
