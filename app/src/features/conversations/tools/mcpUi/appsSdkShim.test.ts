import { describe, expect, it } from 'vitest';

import { APPS_SDK_SHIM_SOURCE, injectScript } from './appsSdkShim';

describe('injectScript', () => {
  it('runs the script before the document content', () => {
    expect(injectScript('<html><head><title>x</title></head></html>', 'A')).toBe(
      '<html><head><script>A</script><title>x</title></head></html>'
    );
    expect(injectScript('<!doctype html><p>hi</p>', 'A')).toBe(
      '<!doctype html><script>A</script><p>hi</p>'
    );
    expect(injectScript('<p>hi</p>', 'A')).toBe('<script>A</script><p>hi</p>');
  });

  it('cannot be closed early by the script body', () => {
    expect(injectScript('<p/>', 'x</script><b>')).toContain('x<\\/script><b>');
  });
});

describe('APPS_SDK_SHIM_SOURCE', () => {
  it('maps window.openai onto MCP Apps messages', () => {
    const posted: Array<Record<string, unknown>> = [];
    const listeners: Array<(event: { source: unknown; data: unknown }) => void> = [];
    const host = { postMessage: (message: Record<string, unknown>) => posted.push(message) };
    const fakeWindow: Record<string, unknown> = {
      parent: host,
      addEventListener: (type: string, fn: (event: { source: unknown; data: unknown }) => void) => {
        if (type === 'message') listeners.push(fn);
      },
      dispatchEvent: () => true,
    };
    const fakeDocument = { documentElement: { scrollHeight: 120 } };
    class FakeEvent {
      constructor(
        readonly type: string,
        readonly init?: unknown
      ) {}
    }
    new Function('window', 'document', 'CustomEvent', 'ResizeObserver', APPS_SDK_SHIM_SOURCE)(
      fakeWindow,
      fakeDocument,
      FakeEvent,
      undefined
    );
    const openai = fakeWindow.openai as {
      toolOutput: unknown;
      callTool: (name: string, args: unknown) => Promise<unknown>;
      openExternal: (o: { href: string }) => Promise<void>;
    };
    expect(posted[0]).toMatchObject({ method: 'ui/initialize' });

    const deliver = (data: unknown) => listeners.forEach(fn => fn({ source: host, data }));
    deliver({
      jsonrpc: '2.0',
      method: 'ui/notifications/tool-result',
      params: { structuredContent: { total: 120 } },
    });
    expect(openai.toolOutput).toEqual({ total: 120 });

    void openai.callTool('add_to_cart', { id: 1 });
    expect(posted.at(-1)).toMatchObject({
      method: 'tools/call',
      params: { name: 'add_to_cart', arguments: { id: 1 } },
    });
    void openai.openExternal({ href: 'https://pay.example.com' });
    expect(posted.at(-1)).toMatchObject({
      method: 'ui/open-link',
      params: { url: 'https://pay.example.com' },
    });

    const before = posted.length;
    listeners.forEach(fn =>
      fn({
        source: {},
        data: { jsonrpc: '2.0', method: 'ui/notifications/tool-input', params: {} },
      })
    );
    expect(posted.length).toBe(before);
  });
});
