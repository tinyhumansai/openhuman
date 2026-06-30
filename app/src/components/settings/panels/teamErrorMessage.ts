import { CoreRpcError } from '../../../services/coreRpcClient';

/**
 * Extract a human-readable failure reason from a team RPC rejection.
 *
 * Team ops call the hosted backend through `callCoreRpc`, which rejects with a
 * {@link CoreRpcError}. The real reason lives in either `err.data` (structured
 * JSON-RPC `error.data`) or `err.message` — the latter shaped by the Rust
 * `flatten_authed_error` net as `"POST /teams/join failed (400 Bad Request):
 * <body>"`. The previous TeamPanel catch checked `'error' in err`, which never
 * matched a `CoreRpcError` (it has no `.error` field), so every failure fell
 * back to a generic banner and the backend reason was dropped (issue #3723).
 *
 * This helper surfaces the backend reason when one is recoverable and falls
 * back to the caller-supplied localized string otherwise. It also handles the
 * legacy plain `{ error }` / `{ message }` rejection shape so existing call
 * sites keep working.
 */

const RPC_PREFIX = /^(?:GET|POST|PUT|DELETE|PATCH)\s+\S+\s+failed\s*\([^)]*\):?\s*/i;
const MAX_LEN = 200;

function cap(value: string): string {
  const oneLine = value.replace(/\s+/g, ' ').trim();
  return oneLine.length > MAX_LEN ? `${oneLine.slice(0, MAX_LEN - 1)}…` : oneLine;
}

function firstString(obj: Record<string, unknown>, keys: string[]): string | null {
  for (const key of keys) {
    const value = obj[key];
    if (typeof value === 'string' && value.trim()) return value.trim();
  }
  return null;
}

/** Pull a human field out of a structured error body / `error.data`. */
function reasonFromData(data: unknown): string | null {
  if (data && typeof data === 'object') {
    const reason = firstString(data as Record<string, unknown>, ['message', 'error', 'detail']);
    if (reason) return cap(reason);
  }
  return null;
}

/**
 * Strip the `"<VERB> <path> failed (<status>):"` prefix the core prepends and
 * recover the meaningful tail. Returns `null` when nothing presentable remains
 * (empty body, or a raw HTML error page that must never reach the user).
 */
function cleanRpcMessage(message: string): string | null {
  const body = message.replace(RPC_PREFIX, '').trim();
  if (!body) return null;
  // Never surface a raw HTML 404/error page (the POST /teams 404 case).
  if (/^<(?:!doctype|html)/i.test(body)) return null;
  // Backend errors arrive as a JSON body — lift the human field out of it.
  if (body.startsWith('{')) {
    try {
      const fromJson = reasonFromData(JSON.parse(body));
      // Structured but no human field → let the caller fall back.
      return fromJson;
    } catch {
      // Not valid JSON — fall through and surface the raw tail.
    }
  }
  return cap(body);
}

export function teamErrorMessage(err: unknown, fallback: string): string {
  if (err instanceof CoreRpcError) {
    return reasonFromData(err.data) ?? cleanRpcMessage(err.message) ?? fallback;
  }
  if (err && typeof err === 'object') {
    const reason = firstString(err as Record<string, unknown>, ['error', 'detail']);
    if (reason) return cap(reason);
    const message = (err as Record<string, unknown>).message;
    if (typeof message === 'string' && message.trim()) {
      return cleanRpcMessage(message) ?? cap(message);
    }
  }
  return fallback;
}
