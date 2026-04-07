/**
 * HTTP JSON-RPC to the desktop core sidecar (same process as the main app).
 * Mirrors the naming normalization in app/src/services/coreRpcClient.ts (subset).
 */

let nextJsonRpcId = 1;

export function normalizeLegacyMethod(method: string): string {
  if (method.startsWith("openhuman.accessibility_")) {
    return method.replace("openhuman.accessibility_", "openhuman.screen_intelligence_");
  }
  return method;
}

/** RpcOutcome with non-empty logs serializes as `{ result, logs }` in the core. */
function unwrapCliCompatibleJson<T>(raw: unknown): T {
  if (
    raw !== null &&
    typeof raw === "object" &&
    "result" in raw &&
    "logs" in raw &&
    Array.isArray((raw as { logs: unknown }).logs)
  ) {
    return (raw as { result: T }).result;
  }
  return raw as T;
}

interface JsonRpcError {
  code: number;
  message: string;
  data?: unknown;
}

interface JsonRpcResponse<T> {
  jsonrpc?: string;
  id?: number | string | null;
  result?: T;
  error?: JsonRpcError;
}

/** Default timeout for parent core RPC requests (ms). */
const DEFAULT_RPC_TIMEOUT_MS = 10_000;

export async function callParentCoreRpc<T>(
  rpcUrl: string,
  method: string,
  params: Record<string, unknown> = {},
  timeoutMs: number = DEFAULT_RPC_TIMEOUT_MS,
): Promise<T> {
  const normalizedMethod = normalizeLegacyMethod(method);
  const payload = {
    jsonrpc: "2.0" as const,
    id: nextJsonRpcId++,
    method: normalizedMethod,
    params,
  };

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);

  let response: Response;
  try {
    response = await fetch(rpcUrl, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
      signal: controller.signal,
    });
  } catch (err) {
    clearTimeout(timer);
    if (err instanceof DOMException && err.name === "AbortError") {
      throw new Error(`Core RPC request timed out after ${timeoutMs}ms (method: ${normalizedMethod})`);
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }

  if (!response.ok) {
    const text = await response.text();
    throw new Error(`Core RPC HTTP ${response.status}: ${text || response.statusText}`);
  }

  const json = (await response.json()) as JsonRpcResponse<unknown>;

  if (json.error) {
    throw new Error(json.error.message || "Core RPC returned an error");
  }
  if (!Object.prototype.hasOwnProperty.call(json, "result")) {
    throw new Error("Core RPC response missing result");
  }

  return unwrapCliCompatibleJson<T>(json.result);
}
