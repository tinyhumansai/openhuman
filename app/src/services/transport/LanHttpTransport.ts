/**
 * LanHttpTransport — HTTP transport pointing at a rpcUrl from a Connection profile.
 *
 * Same JSON-RPC wire format as LocalTransport, but no bearer token (LAN
 * connections rely on network-level trust + the session token in the profile).
 */
import debug from 'debug';

import type { CoreTransport } from './CoreTransport';

const log = debug('transport:lan');
const logErr = debug('transport:lan:error');

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

export class LanHttpTransport implements CoreTransport {
  readonly kind = 'lan-http' as const;

  constructor(
    private readonly rpcUrl: string,
    private readonly timeoutMs: number = 10_000
  ) {
    log('[transport:lan] created rpcUrl=%s', rpcUrl);
  }

  async call<T>(method: string, params: unknown, opts?: { signal?: AbortSignal }): Promise<T> {
    const id = _nextId++;
    const payload: JsonRpcRequestBody = { jsonrpc: '2.0', id, method, params: params ?? {} };

    log('[transport:lan] → %s id=%d', method, id);

    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), this.timeoutMs);
    opts?.signal?.addEventListener('abort', () => controller.abort());

    let response: Response;
    try {
      response = await fetch(this.rpcUrl, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
        signal: controller.signal,
      });
    } catch (err) {
      if (controller.signal.aborted) {
        throw new Error(`[transport:lan] ${method} timed out after ${this.timeoutMs}ms`);
      }
      throw err;
    } finally {
      clearTimeout(timeoutId);
    }

    if (!response.ok) {
      const text = await response.text();
      throw new Error(`[transport:lan] HTTP ${response.status}: ${text || response.statusText}`);
    }

    const json = (await response.json()) as JsonRpcResponse<T>;

    if (json.error) {
      logErr('[transport:lan] ← %s error: %s', method, json.error.message);
      throw new Error(json.error.message ?? 'LAN RPC returned an error');
    }
    if (!Object.prototype.hasOwnProperty.call(json, 'result')) {
      throw new Error('[transport:lan] response missing result');
    }

    log('[transport:lan] ← %s id=%d ok', method, id);
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
      await this.call('openhuman.ping', {}, { signal: AbortSignal.timeout(2000) });
      return true;
    } catch {
      return false;
    }
  }

  async close(): Promise<void> {
    log('[transport:lan] close (no-op)');
  }
}
