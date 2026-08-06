import { afterEach, describe, expect, it, vi } from 'vitest';

import { formatRpcCallFailure } from './e2e/helpers/core-rpc-node';

describe('formatRpcCallFailure', () => {
  it('includes the RPC method, status, and error text', () => {
    expect(
      formatRpcCallFailure('openhuman.composio_list_triggers', {
        ok: false,
        httpStatus: 500,
        error: 'Backend returned 500: trigger store unavailable',
      })
    ).toContain(
      'openhuman.composio_list_triggers failed: httpStatus=500 error=Backend returned 500: trigger store unavailable'
    );
  });
});

describe('callOpenhumanRpcNode', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.resetModules();
  });

  it('rediscovers the core when the cached listener disappears after a reset', async () => {
    const requestedUrls: string[] = [];
    let firstListenerAlive = true;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
        const url = String(input);
        requestedUrls.push(url);
        const body = JSON.parse(String(init?.body)) as { method: string };

        if (url.includes(':7788/')) {
          if (body.method === 'core.ping' && firstListenerAlive) {
            return new Response('', { status: 401 });
          }
          if (body.method === 'openhuman.first_call' && firstListenerAlive) {
            return Response.json({ result: 'first' });
          }
          throw new TypeError('fetch failed');
        }

        if (url.includes(':7789/')) {
          if (body.method === 'core.ping') return new Response('', { status: 401 });
          return Response.json({ result: 'replacement' });
        }

        throw new TypeError('fetch failed');
      })
    );

    const { callOpenhumanRpcNode } = await import('./e2e/helpers/core-rpc-node');
    await expect(callOpenhumanRpcNode('openhuman.first_call')).resolves.toMatchObject({
      ok: true,
      result: 'first',
    });

    firstListenerAlive = false;
    await expect(callOpenhumanRpcNode('openhuman.after_reset')).resolves.toMatchObject({
      ok: true,
      result: 'replacement',
    });
    expect(requestedUrls.some(url => url.includes(':7789/rpc'))).toBe(true);
  });
});
