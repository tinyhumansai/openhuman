/**
 * LocalTransport — wraps the existing local-spawn HTTP path.
 *
 * This is the transport used on desktop when the core sidecar is running
 * locally. It delegates all RPC logic to the getCoreRpcUrl / getCoreRpcToken
 * resolution that already lives in coreRpcClient.ts.
 */
import debug from 'debug';

import type { CoreTransport } from './CoreTransport';

const log = debug('transport:local');
const logErr = debug('transport:local:error');

interface JsonRpcRequestBody {
  jsonrpc: '2.0';
  id: number;
  method: string;
  params: unknown;
}

interface JsonRpcResponse<T> {
  jsonrpc?: string;
  id?: number | string | null;
  result?: T;
  error?: { code: number; message: string; data?: unknown };
}

let _nextId = 1;

export class LocalTransport implements CoreTransport {
  readonly kind = 'local' as const;

  constructor(
    private readonly getRpcUrl: () => Promise<string>,
    private readonly getToken: () => Promise<string | null>,
    private readonly timeoutMs: number = 30_000
  ) {}

  async call<T>(method: string, params: unknown, opts?: { signal?: AbortSignal }): Promise<T> {
    const id = _nextId++;
    const payload: JsonRpcRequestBody = { jsonrpc: '2.0', id, method, params: params ?? {} };

    const [rpcUrl, token] = await Promise.all([this.getRpcUrl(), this.getToken()]);
    log('[transport:local] → %s id=%d', method, id);

    const headers: Record<string, string> = { 'Content-Type': 'application/json' };
    if (token) {
      headers.Authorization = `Bearer ${token}`;
    }

    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), this.timeoutMs);

    // Merge caller signal with timeout signal.
    opts?.signal?.addEventListener('abort', () => controller.abort());

    let response: Response;
    try {
      response = await fetch(rpcUrl, {
        method: 'POST',
        headers,
        body: JSON.stringify(payload),
        signal: controller.signal,
      });
    } catch (err) {
      if (controller.signal.aborted) {
        throw new Error(`[transport:local] ${method} timed out after ${this.timeoutMs}ms`);
      }
      throw err;
    } finally {
      clearTimeout(timeoutId);
    }

    if (!response.ok) {
      const text = await response.text();
      throw new Error(`[transport:local] HTTP ${response.status}: ${text || response.statusText}`);
    }

    const json = (await response.json()) as JsonRpcResponse<T>;

    if (json.error) {
      logErr('[transport:local] ← %s error: %s', method, json.error.message);
      throw new Error(json.error.message ?? 'Core RPC returned an error');
    }
    if (!Object.prototype.hasOwnProperty.call(json, 'result')) {
      throw new Error('[transport:local] response missing result');
    }

    log('[transport:local] ← %s id=%d ok', method, id);
    return json.result as T;
  }

  async *stream<T>(
    method: string,
    params: unknown,
    opts?: { signal?: AbortSignal }
  ): AsyncIterable<T> {
    // Local HTTP doesn't support streaming natively in this project.
    // Fall back to a single call and yield the result.
    const result = await this.call<T>(method, params, opts);
    yield result;
  }

  async isHealthy(): Promise<boolean> {
    try {
      await this.call('openhuman.ping', {}, { signal: AbortSignal.timeout(3000) });
      return true;
    } catch {
      return false;
    }
  }

  async close(): Promise<void> {
    // Stateless HTTP — nothing to tear down.
    log('[transport:local] close (no-op)');
  }
}
