import { callCoreRpc } from '../coreRpcClient';
import { CORE_RPC_METHODS } from '../rpcMethods';

export type CoreApprovalState =
  | 'not_required'
  | 'pending'
  | 'approved'
  | 'rejected'
  | 'expired'
  | (string & {});

export type CoreExecutionState =
  | 'not_started'
  | 'queued'
  | 'running'
  | 'succeeded'
  | 'failed'
  | 'cancelled'
  | (string & {});

export interface CoreActionRequestLifecycleEnvelope {
  action_request: Record<string, unknown>;
  row_version: number;
  id: string;
  tenant_id: string;
  approval_state: CoreApprovalState;
  execution_state: CoreExecutionState;
  policy_outcome: string;
  correlation_id: string;
  created_at: string;
  updated_at: string;
}

export interface ListCoreActionRequestsParams {
  tenantId?: string;
  approvalState?: CoreApprovalState;
  executionState?: CoreExecutionState;
  limit?: number;
}

export interface CoreActionRequestDecisionParams {
  reason: string;
  expectedRowVersion: number;
  /** Required stable per-intent key so retries are replay-safe. */
  idempotencyKey: string;
}

export interface CoreActionRequestClientOptions {
  timeoutMs?: number;
}

type CoreResult<T> = T | { result: T; logs?: string[] };

export class CoreActionRequestClient {
  private readonly timeoutMs?: number;

  constructor(options: CoreActionRequestClientOptions = {}) {
    this.timeoutMs = options.timeoutMs;
  }

  async list(
    params: ListCoreActionRequestsParams = {}
  ): Promise<CoreActionRequestLifecycleEnvelope[]> {
    const raw = await callCoreRpc<CoreResult<CoreActionRequestLifecycleEnvelope[]>>({
      method: CORE_RPC_METHODS.youpetListActionRequests,
      params,
      timeoutMs: this.timeoutMs,
    });
    return unwrapCoreResult(raw);
  }

  async get(actionRequestId: string): Promise<CoreActionRequestLifecycleEnvelope> {
    const raw = await callCoreRpc<CoreResult<CoreActionRequestLifecycleEnvelope>>({
      method: CORE_RPC_METHODS.youpetGetActionRequest,
      params: { actionRequestId },
      timeoutMs: this.timeoutMs,
    });
    return unwrapCoreResult(raw);
  }

  async approve(
    actionRequestId: string,
    params: CoreActionRequestDecisionParams
  ): Promise<CoreActionRequestLifecycleEnvelope> {
    assertDecisionParams(params);
    const raw = await callCoreRpc<CoreResult<CoreActionRequestLifecycleEnvelope>>({
      method: CORE_RPC_METHODS.youpetApproveActionRequest,
      params: {
        actionRequestId,
        reason: params.reason,
        expectedRowVersion: params.expectedRowVersion,
        idempotencyKey: params.idempotencyKey,
      },
      timeoutMs: this.timeoutMs,
    });
    return unwrapCoreResult(raw);
  }

  async reject(
    actionRequestId: string,
    params: CoreActionRequestDecisionParams
  ): Promise<CoreActionRequestLifecycleEnvelope> {
    assertDecisionParams(params);
    const raw = await callCoreRpc<CoreResult<CoreActionRequestLifecycleEnvelope>>({
      method: CORE_RPC_METHODS.youpetRejectActionRequest,
      params: {
        actionRequestId,
        reason: params.reason,
        expectedRowVersion: params.expectedRowVersion,
        idempotencyKey: params.idempotencyKey,
      },
      timeoutMs: this.timeoutMs,
    });
    return unwrapCoreResult(raw);
  }
}

function assertDecisionParams(params: CoreActionRequestDecisionParams) {
  if (!params.reason?.trim()) {
    throw new Error('reason is required for ActionRequest decisions');
  }
  if (!Number.isFinite(params.expectedRowVersion) || params.expectedRowVersion < 1) {
    throw new Error('expectedRowVersion must be >= 1');
  }
  if (!params.idempotencyKey?.trim()) {
    throw new Error('idempotencyKey is required for ActionRequest decisions');
  }
}

export const createCoreActionRequestClient = (
  options: CoreActionRequestClientOptions = {}
): CoreActionRequestClient => new CoreActionRequestClient(options);

function unwrapCoreResult<T>(value: CoreResult<T>): T {
  if (isRecord(value) && 'result' in value) {
    return value.result as T;
  }
  return value as T;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function youpetPayload(error: unknown): Record<string, unknown> | null {
  if (!error || typeof error !== 'object') return null;
  const data = (error as { data?: unknown }).data;
  if (!isRecord(data)) return null;
  const youpet = data.youpet;
  return isRecord(youpet) ? youpet : null;
}

/** Extract a stable Core lifecycle error code from structured RPC failures when present. */
export function extractYoupetErrorCode(error: unknown): string | null {
  const youpet = youpetPayload(error);
  if (!youpet) return null;
  const code = youpet.code;
  return typeof code === 'string' && code.trim() ? code : null;
}

/** Extract a structured config/request field marker (e.g. tenant_id) when present. */
export function extractYoupetErrorField(error: unknown): string | null {
  const youpet = youpetPayload(error);
  if (!youpet) return null;
  const field = youpet.field;
  return typeof field === 'string' && field.trim() ? field : null;
}
