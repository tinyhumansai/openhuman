/**
 * Imperative RPC wrapper for the Recall.ai Calendar domain — typed counterpart
 * to `src/openhuman/recall_calendar/*` on the Rust side.
 *
 * Every function calls the core via JSON-RPC. The core proxies to the
 * openhuman backend's `/agent-integrations/recall-calendar/*` routes, so the
 * frontend never talks to Recall.ai directly and never handles the API key.
 */
import { callCoreRpc } from '../../services/coreRpcClient';

/**
 * Each `recall_calendar_*` op returns an `RpcOutcome` with a user-visible log
 * line, wrapped by `into_cli_compatible_json` as `{ result, logs }`. Peel that
 * envelope back off so callers work with the flat shapes. Mirrors the helper
 * in `lib/composio/composioApi.ts`.
 */
function unwrapCliEnvelope<T>(value: unknown): T {
  if (
    value !== null &&
    typeof value === 'object' &&
    'result' in (value as Record<string, unknown>) &&
    'logs' in (value as Record<string, unknown>) &&
    Array.isArray((value as { logs: unknown }).logs)
  ) {
    return (value as { result: T }).result;
  }
  return value as T;
}

export interface RecallCalendarStatus {
  /** Whether the backend has the Recall calendar path enabled. */
  enabled: boolean;
  connected: boolean;
  email?: string;
}

export interface RecallCalendarConnect {
  connectUrl: string;
}

export interface RecallCalendarDisconnect {
  disconnected: boolean;
}

/** Start the OAuth flow; returns the Google consent URL to open in a browser. */
export async function connect(): Promise<RecallCalendarConnect> {
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.recall_calendar_connect' });
  return unwrapCliEnvelope<RecallCalendarConnect>(raw);
}

/** Current connection status for the settings UI. */
export async function status(): Promise<RecallCalendarStatus> {
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.recall_calendar_status' });
  return unwrapCliEnvelope<RecallCalendarStatus>(raw);
}

/** Disconnect the user's Google Calendar from Recall. */
export async function disconnect(): Promise<RecallCalendarDisconnect> {
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.recall_calendar_disconnect' });
  return unwrapCliEnvelope<RecallCalendarDisconnect>(raw);
}
