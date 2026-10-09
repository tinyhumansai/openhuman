import { describe, expect, it, vi } from 'vitest';

import {
  type BridgeHandlers,
  MAX_FRAME_HEIGHT,
  McpAppBridge,
  readRpcMessage,
} from './mcpAppBridge';
import { SCROLL_AFFORDANCE_SOURCE } from './scrollAffordance';
import type { McpUiPresentation } from './types';

const flush = () => new Promise(resolve => setTimeout(resolve, 0));

function setup(
  overrides: Partial<McpUiPresentation> = {},
  handlerOverrides: Partial<BridgeHandlers> = {}
) {
  const posted: Array<Record<string, unknown>> = [];
  const frame = {
    postMessage: (message: Record<string, unknown>) => posted.push(message),
  } as unknown as Window;
  const handlers: BridgeHandlers = {
    callTool: vi.fn(async () => ({ content: [], structuredContent: { ok: true } })),
    openExternal: vi.fn(async () => undefined),
    handOff: vi.fn(),
    prefillMessage: vi.fn(),
    readResource: vi.fn(async () => ({ contents: [] })),
    resize: vi.fn(),
    fitWidth: vi.fn(),
    ...handlerOverrides,
  };
  let clock = 0;
  const bridge = new McpAppBridge({
    presentation: {
      kind: 'mcp_ui',
      flavor: 'mcp_apps',
      server_id: 'srv',
      tool: 'cart',
      resource_uri: 'ui://cart',
      tool_input: { item: 'dosa' },
      structured_content: { total: 120 },
      links: [],
      ...overrides,
    },
    html: '<html><head></head><body>cart</body></html>',
    theme: 'dark',
    locale: 'en',
    handlers,
    source: () => frame,
    now: () => clock,
  });
  const send = (data: unknown, source: unknown = frame) =>
    bridge.handleMessage({ source, data } as unknown as MessageEvent);
  return { bridge, posted, handlers, send, frame, tick: (ms: number) => (clock += ms) };
}

describe('readRpcMessage', () => {
  it('accepts JSON-RPC requests and notifications only', () => {
    expect(readRpcMessage({ jsonrpc: '2.0', method: 'ping', id: 1 })).not.toBeNull();
    expect(readRpcMessage({ jsonrpc: '2.0', method: 'x' })).not.toBeNull();
    expect(readRpcMessage({ method: 'ping', id: 1 })).toBeNull();
    expect(readRpcMessage({ jsonrpc: '2.0', method: 'x', params: [1] })).toBeNull();
    expect(readRpcMessage({ jsonrpc: '2.0', method: 'x', id: {} })).toBeNull();
    expect(readRpcMessage('{"jsonrpc":"2.0"}')).toBeNull();
  });
});

describe('McpAppBridge', () => {
  it('ignores messages from any other window', async () => {
    const { send, posted } = setup();
    expect(send({ jsonrpc: '2.0', method: 'ping', id: 1 }, window)).toBe(false);
    await flush();
    expect(posted).toHaveLength(0);
  });

  it('delivers the document once the proxy is ready, once', () => {
    const { send, posted } = setup();
    send({ jsonrpc: '2.0', method: 'ui/notifications/sandbox-proxy-ready', params: {} });
    send({ jsonrpc: '2.0', method: 'ui/notifications/sandbox-proxy-ready', params: {} });
    expect(posted).toHaveLength(1);
    expect(posted[0]).toMatchObject({ method: 'ui/notifications/sandbox-resource-ready' });
    const html = (posted[0].params as { html: string }).html;
    expect(html.startsWith('<html><head><script>')).toBe(true);
    expect(html.endsWith('</head><body>cart</body></html>')).toBe(true);
  });

  it('injects the host scripts for both flavors, the shim only for Apps SDK', () => {
    for (const flavor of ['mcp_apps', 'apps_sdk'] as const) {
      const { send, posted } = setup({ flavor });
      send({ jsonrpc: '2.0', method: 'ui/notifications/sandbox-proxy-ready' });
      const html = (posted[0].params as { html: string }).html;
      expect(html).toContain(SCROLL_AFFORDANCE_SOURCE.slice(0, 40));
      expect(html).toContain('html,body{background:transparent!important;}');
      expect(html).toContain("var THEME = 'dark';");
      expect(html.includes('window.openai')).toBe(flavor === 'apps_sdk');
      expect(html.lastIndexOf('</script>')).toBeLessThan(html.indexOf('cart'));
    }
  });

  it('injects the Apps SDK shim for Apps SDK widgets', () => {
    const { send, posted } = setup({ flavor: 'apps_sdk' });
    send({ jsonrpc: '2.0', method: 'ui/notifications/sandbox-proxy-ready' });
    const html = (posted[0].params as { html: string }).html;
    expect(html).toContain('window.openai');
    expect(html.indexOf('<script>')).toBeLessThan(html.indexOf('cart'));
  });

  it('answers ui/initialize with host context, then sends tool input and result', async () => {
    const { send, posted } = setup();
    send({ jsonrpc: '2.0', id: 1, method: 'ui/initialize', params: { protocolVersion: 'v1' } });
    await flush();
    expect(posted[0]).toMatchObject({
      id: 1,
      result: { protocolVersion: 'v1', hostContext: { theme: 'dark', displayMode: 'inline' } },
    });
    send({ jsonrpc: '2.0', method: 'ui/notifications/initialized' });
    expect(posted[1]).toMatchObject({
      method: 'ui/notifications/tool-input',
      params: { arguments: { item: 'dosa' } },
    });
    expect(posted[2]).toMatchObject({
      method: 'ui/notifications/tool-result',
      params: { structuredContent: { total: 120 } },
    });
  });

  it('sends newer data to an initialized widget, once per change', () => {
    const { bridge, send, posted } = setup();
    const base: McpUiPresentation = {
      kind: 'mcp_ui',
      flavor: 'mcp_apps',
      server_id: 'srv',
      tool: 'cart',
      resource_uri: 'ui://cart',
      links: [],
    };
    bridge.updatePresentation({ ...base, structured_content: { total: 150 } });
    expect(posted).toHaveLength(0);

    send({ jsonrpc: '2.0', method: 'ui/notifications/initialized' });
    expect(posted.at(-1)).toMatchObject({ params: { structuredContent: { total: 150 } } });

    const latest = { total: 200 };
    bridge.updatePresentation({ ...base, structured_content: latest });
    expect(posted.at(-1)).toMatchObject({
      method: 'ui/notifications/tool-result',
      params: { structuredContent: { total: 200 } },
    });
    const count = posted.length;
    bridge.updatePresentation({ ...base, structured_content: latest });
    expect(posted).toHaveLength(count);
  });

  it('routes tools/call through the handler', async () => {
    const { send, posted, handlers } = setup();
    send({
      jsonrpc: '2.0',
      id: 'a',
      method: 'tools/call',
      params: { name: 'add', arguments: { id: 2 } },
    });
    await flush();
    expect(handlers.callTool).toHaveBeenCalledWith('add', { id: 2 });
    expect(posted[0]).toMatchObject({ id: 'a', result: { structuredContent: { ok: true } } });
  });

  it('reports a declined tool call as an error', async () => {
    const { send, posted } = setup(
      {},
      {
        callTool: async () => {
          throw new Error('The user declined this action');
        },
      }
    );
    send({ jsonrpc: '2.0', id: 2, method: 'tools/call', params: { name: 'pay' } });
    await flush();
    expect(posted[0]).toMatchObject({ id: 2, error: { message: 'The user declined this action' } });
  });

  it('refuses tool calls and resource reads from host views', async () => {
    const { send, posted, handlers } = setup({ flavor: 'host_inline', server_id: undefined });
    send({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'x' } });
    send({ jsonrpc: '2.0', id: 2, method: 'resources/read', params: { uri: 'ui://x' } });
    await flush();
    expect(handlers.callTool).not.toHaveBeenCalled();
    expect(handlers.readResource).not.toHaveBeenCalled();
    expect(posted.every(message => 'error' in message)).toBe(true);
  });

  it('opens https links, hands off app links, and refuses the rest', async () => {
    const { send, posted, handlers, tick } = setup();
    send({
      jsonrpc: '2.0',
      id: 1,
      method: 'ui/open-link',
      params: { url: 'https://pay.example.com' },
    });
    send({ jsonrpc: '2.0', id: 2, method: 'ui/open-link', params: { url: 'phonepe://pay?id=1' } });
    send({ jsonrpc: '2.0', id: 3, method: 'ui/open-link', params: { url: 'javascript:alert(1)' } });
    await flush();
    expect(handlers.openExternal).toHaveBeenCalledWith('https://pay.example.com');
    expect(handlers.handOff).toHaveBeenCalledWith('phonepe://pay?id=1');
    expect(posted.find(message => message.id === 3)).toHaveProperty('error');

    send({
      jsonrpc: '2.0',
      id: 4,
      method: 'ui/open-link',
      params: { url: 'https://b.example.com' },
    });
    await flush();
    expect(posted.find(message => message.id === 4)).toHaveProperty('error');
    tick(1500);
    send({
      jsonrpc: '2.0',
      id: 5,
      method: 'ui/open-link',
      params: { url: 'https://b.example.com' },
    });
    await flush();
    expect(handlers.openExternal).toHaveBeenCalledTimes(2);
  });

  it('prefills the composer from ui/message without sending', async () => {
    const { send, handlers } = setup();
    send({
      jsonrpc: '2.0',
      id: 1,
      method: 'ui/message',
      params: { role: 'user', content: [{ type: 'text', text: 'Checkout please' }] },
    });
    await flush();
    expect(handlers.prefillMessage).toHaveBeenCalledWith('Checkout please');
  });

  it('only reads ui:// resources', async () => {
    const { send, handlers, posted } = setup();
    send({
      jsonrpc: '2.0',
      id: 1,
      method: 'resources/read',
      params: { uri: 'file:///etc/passwd' },
    });
    send({ jsonrpc: '2.0', id: 2, method: 'resources/read', params: { uri: 'ui://cart/2' } });
    await flush();
    expect(handlers.readResource).toHaveBeenCalledTimes(1);
    expect(handlers.readResource).toHaveBeenCalledWith('ui://cart/2');
    expect(posted.find(message => message.id === 1)).toHaveProperty('error');
  });

  it('clamps size changes', () => {
    const { send, handlers } = setup();
    send({ jsonrpc: '2.0', method: 'ui/notifications/size-changed', params: { height: 99999 } });
    expect(handlers.resize).toHaveBeenCalledWith(MAX_FRAME_HEIGHT);
  });

  it('takes width only from the host fit report, height only from the widget', () => {
    const { send, handlers } = setup();
    send({
      jsonrpc: '2.0',
      method: 'ui/notifications/size-changed',
      params: { width: 480.4, height: 900, fit: true },
    });
    expect(handlers.fitWidth).toHaveBeenCalledWith(481);
    expect(handlers.resize).not.toHaveBeenCalled();

    send({
      jsonrpc: '2.0',
      method: 'ui/notifications/size-changed',
      params: { width: 300, height: 200 },
    });
    expect(handlers.fitWidth).toHaveBeenCalledTimes(1);
    expect(handlers.resize).toHaveBeenCalledWith(200);

    send({
      jsonrpc: '2.0',
      method: 'ui/notifications/size-changed',
      params: { width: 'x', fit: true },
    });
    expect(handlers.fitWidth).toHaveBeenCalledTimes(1);
  });

  it('rejects unknown requests', async () => {
    const { send, posted } = setup();
    send({ jsonrpc: '2.0', id: 9, method: 'ui/eval', params: {} });
    await flush();
    expect(posted[0]).toMatchObject({ id: 9, error: { code: -32601 } });
  });
});
