// @ts-nocheck
/** Shared result shape for the Node-side E2E core RPC client. */

export interface RpcCallResult<T = unknown> {
  ok: boolean;
  httpStatus?: number;
  error?: string;
  result?: T;
}
