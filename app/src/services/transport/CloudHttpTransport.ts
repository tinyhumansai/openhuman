/**
 * CloudHttpTransport — HTTP transport for user-configured cloud cores.
 *
 * Identical wire format to LanHttpTransport but uses a different auth header
 * (Bearer token from the connection profile) and a longer default timeout.
 */
import debug from 'debug';

import type { CoreTransport } from './CoreTransport';

const log = debug('transport:cloud');
const logErr = debug('transport:cloud:error');

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

export class CloudHttpTransport implements CoreTransport {
  readonly kind = 'cloud-http' as const;

  constructor(
    private readonly rpcUrl: string,
    private readonly bearerToken: string | null = null,
    private readonly timeoutMs: number = 30_000
  ) {
    log('[transport:cloud] created rpcUrl=%s token=%s', rpcUrl, bearerToken ? 'set' : 'none');
  }

  async call<T>(method: string, params: unknown, opts?: { signal?: AbortSignal }): Promise<T> {
    const id = _nextId++;
    const payload: JsonRpcRequestBody = { jsonrpc: '2.0', id, method, params: params ?? {} };

    log('[transport:cloud] → %s id=%d', method, id);

    const headers: Record<string, string> = { 'Content-Type': 'application/json' };
    if (this.bearerToken) {
      headers.Authorization = `Bearer ${this.bearerToken}`;
    }

    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), this.timeoutMs);
    opts?.signal?.addEventListener('abort', () => controller.abort());

    let response: Response;
    try {
      response = await fetch(this.rpcUrl, {
        method: 'POST',
        headers,
        body: JSON.stringify(payload),
        signal: controller.signal,
      });
    } catch (err) {
      if (controller.signal.aborted) {
        throw new Error(`[transport:cloud] ${method} timed out after ${this.timeoutMs}ms`);
      }
      throw err;
    } finally {
      clearTimeout(timeoutId);
    }

    if (!response.ok) {
      const text = await response.text();
      throw new Error(`[transport:cloud] HTTP ${response.status}: ${text || response.statusText}`);
    }

    const json = (await response.json()) as JsonRpcResponse<T>;

    if (json.error) {
      logErr('[transport:cloud] ← %s error: %s', method, json.error.message);
      throw new Error(json.error.message ?? 'Cloud RPC returned an error');
    }
    if (!Object.prototype.hasOwnProperty.call(json, 'result')) {
      throw new Error('[transport:cloud] response missing result');
    }

    log('[transport:cloud] ← %s id=%d ok', method, id);
    return json.result as T;
  }

  async *stream<T>(
    method: string,
    params: unknown,
    opts?: { signal?: AbortSignal }
  ): AsyncIterable<T> {
    const result = await this.call<T>(method, params, opts);
    yield result;
  }

  async isHealthy(): Promise<boolean> {
    try {
      await this.call('openhuman.ping', {}, { signal: AbortSignal.timeout(5000) });
      return true;
    } catch {
      return false;
    }
  }

  async close(): Promise<void> {
    log('[transport:cloud] close (no-op)');
  }
}
