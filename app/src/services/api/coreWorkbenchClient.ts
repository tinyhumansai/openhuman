import { callCoreRpc } from '../coreRpcClient';
import { CORE_RPC_METHODS } from '../rpcMethods';

export type CoreAlertSeverity = 'low' | 'medium' | 'high' | 'critical';
export type CoreAlertStatus = 'open' | 'acknowledged' | 'resolved' | 'dismissed';
type FutureCoreLiteral = string & {};
export type CoreWorkbenchTraceEntryKind =
  | 'alert_created'
  | 'health_plan_state'
  | 'task_state'
  | 'checkin_received'
  | 'action_request_proposed'
  | 'action_request_approved'
  | 'action_request_rejected'
  | 'action_request_execution'
  | 'audit_action'
  | 'outbox_event'
  | 'outbox_delivery'
  | 'delivery_failed'
  | 'delivery_succeeded'
  | 'delivery_recovered'
  | 'delivery_dead_lettered'
  | FutureCoreLiteral;
export type CoreWorkbenchTraceSource =
  | 'alerts'
  | 'health_plans'
  | 'task_instances'
  | 'checkins'
  | 'action_requests'
  | 'audit_logs'
  | 'event_outbox'
  | 'outbox_deliveries'
  | FutureCoreLiteral;
export type CoreWorkbenchTraceWarningCode =
  | 'trace_truncated'
  | 'unsupported_related_type'
  | 'missing_related_task'
  | 'missing_related_plan'
  | 'missing_related_event'
  | 'missing_related_action_request'
  | 'action_request_projection_truncated'
  | 'invalid_action_request_projection'
  | 'action_request_audits_truncated'
  | 'action_request_events_truncated'
  | 'action_request_deliveries_truncated'
  | 'action_request_links_truncated'
  | 'trace_reserved_budget_exceeded'
  | FutureCoreLiteral;
export type CoreWorkbenchTraceSeverity = CoreAlertSeverity | FutureCoreLiteral;

export interface CoreWorkbenchAlertContext {
  pet: { id: string; name: string; species: string; breed?: string | null; status: string };
  owner: { id: string; name: string; phone?: string | null; status: string };
  health_plan: {
    id: string;
    title: string;
    plan_type: string;
    status: string;
    openclaw_flow_id?: string | null;
  };
  task: {
    id: string;
    status: string;
    due_at: string;
    missed_count: number;
    openclaw_flow_id?: string | null;
  };
  latest_checkin?: {
    id: string;
    submitted_at: string;
    submitted_by?: string | null;
    text?: string | null;
    status_tags: string[];
  } | null;
}

export interface CoreWorkbenchAlert {
  id: string;
  alert_type: string;
  severity: CoreAlertSeverity;
  related_type: string;
  related_id: string;
  status: CoreAlertStatus;
  assigned_to?: string | null;
  summary?: string | null;
  created_at: string;
  acknowledged_at?: string | null;
  resolved_at?: string | null;
  context: CoreWorkbenchAlertContext | null;
}

export type CoreWorkbenchAlertActionResult = Omit<CoreWorkbenchAlert, 'context'> & {
  context?: CoreWorkbenchAlertContext | null;
};

export interface CoreWorkbenchTraceActor {
  type: string;
  id?: string | null;
}

export interface CoreWorkbenchTraceEntry {
  id: string;
  occurred_at: string;
  kind: CoreWorkbenchTraceEntryKind;
  source: CoreWorkbenchTraceSource;
  title: string;
  detail?: string | null;
  actor?: CoreWorkbenchTraceActor | null;
  related_type?: string | null;
  related_id?: string | null;
  severity?: CoreWorkbenchTraceSeverity | null;
  metadata: Record<string, unknown>;
}

export interface CoreWorkbenchTraceWarning {
  code: CoreWorkbenchTraceWarningCode;
  message: string;
  source?: CoreWorkbenchTraceSource | null;
}

export interface CoreWorkbenchWorkflowIdentity {
  type: 'health_plan' | FutureCoreLiteral;
  id: string;
  task_id?: string | null;
  openclaw_flow_id?: string | null;
}

export interface CoreWorkbenchAlertTrace {
  alert_id: string;
  workflow: CoreWorkbenchWorkflowIdentity | null;
  partial: boolean;
  warnings: CoreWorkbenchTraceWarning[];
  entries: CoreWorkbenchTraceEntry[];
}

export interface ListCoreWorkbenchAlertsParams {
  /**
   * Omitted uses Core's default open-only filter. `null` requests all states;
   * the Rust bridge also treats an empty string as all states.
   */
  status?: CoreAlertStatus | null;
  severity?: CoreAlertSeverity;
}

export interface CoreAlertActionParams {
  note?: string;
  resolution?: string;
  idempotencyKey?: string;
}

export interface CoreWorkbenchClientOptions {
  timeoutMs?: number;
}

type CoreResult<T> = T | { result: T; logs?: string[] };

export class CoreWorkbenchClient {
  private readonly timeoutMs?: number;

  constructor(options: CoreWorkbenchClientOptions = {}) {
    this.timeoutMs = options.timeoutMs;
  }

  async listAlerts(params: ListCoreWorkbenchAlertsParams = {}): Promise<CoreWorkbenchAlert[]> {
    const raw = await callCoreRpc<CoreResult<CoreWorkbenchAlert[]>>({
      method: CORE_RPC_METHODS.youpetListAlerts,
      params,
      timeoutMs: this.timeoutMs,
    });
    return unwrapCoreResult(raw);
  }

  async ackAlert(
    alertId: string,
    params: CoreAlertActionParams
  ): Promise<CoreWorkbenchAlertActionResult> {
    const raw = await callCoreRpc<CoreResult<CoreWorkbenchAlertActionResult>>({
      method: CORE_RPC_METHODS.youpetAckAlert,
      params: { alertId, note: params.note, idempotencyKey: params.idempotencyKey },
      timeoutMs: this.timeoutMs,
    });
    return unwrapCoreResult(raw);
  }

  async resolveAlert(
    alertId: string,
    params: CoreAlertActionParams
  ): Promise<CoreWorkbenchAlertActionResult> {
    const raw = await callCoreRpc<CoreResult<CoreWorkbenchAlertActionResult>>({
      method: CORE_RPC_METHODS.youpetResolveAlert,
      params: { alertId, resolution: params.resolution, idempotencyKey: params.idempotencyKey },
      timeoutMs: this.timeoutMs,
    });
    return unwrapCoreResult(raw);
  }

  async getAlertTrace(alertId: string): Promise<CoreWorkbenchAlertTrace> {
    const raw = await callCoreRpc<CoreResult<CoreWorkbenchAlertTrace>>({
      method: CORE_RPC_METHODS.youpetTraceAlert,
      params: { alertId },
      timeoutMs: this.timeoutMs,
    });
    return unwrapCoreResult(raw);
  }
}

export const createCoreWorkbenchClient = (
  options: CoreWorkbenchClientOptions = {}
): CoreWorkbenchClient => new CoreWorkbenchClient(options);

function unwrapCoreResult<T>(value: CoreResult<T>): T {
  if (isRecord(value) && 'result' in value) {
    return value.result as T;
  }
  return value as T;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}
