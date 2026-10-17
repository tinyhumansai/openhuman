/**
 * The host side of the MCP Apps channel to one widget frame.
 *
 * The frame is the `ohwidget:` sandbox proxy, which relays JSON-RPC between
 * this window and the widget's inner sandboxed frame. Every message is
 * accepted only from that frame's window and only in JSON-RPC 2.0 shape; what
 * a widget may ask for is decided here and in the handlers, never by the
 * widget.
 */
import debug from 'debug';

import { classifyHref } from '../../utils/format';
import { APPS_SDK_SHIM_SOURCE, injectScript } from './appsSdkShim';
import { frameFitSource } from './frameFit';
import { SCROLL_AFFORDANCE_SOURCE } from './scrollAffordance';
import type { McpUiPresentation } from './types';

const log = debug('mcp-ui:bridge');

export const MAX_FRAME_HEIGHT = 720;
export const MIN_FRAME_HEIGHT = 48;
const MAX_METHOD_LENGTH = 96;
const OPEN_LINK_INTERVAL_MS = 1000;
const PROTOCOL_VERSION = '2025-06-18';

export const RPC_METHOD_NOT_FOUND = -32601;
export const RPC_INVALID_PARAMS = -32602;
export const RPC_INTERNAL_ERROR = -32603;
export const RPC_DENIED = -32000;

export interface BridgeHandlers {
  /** Runs a tool call the widget asked for; rejects when refused. */
  callTool(name: string, args: Record<string, unknown>): Promise<unknown>;
  /** Opens an `http(s)` link in the browser. */
  openExternal(url: string): Promise<void>;
  /** Hands an app link (`upi://`, ...) to the user's phone. */
  handOff(url: string): void;
  /** Prefills the composer; never sends. */
  prefillMessage(text: string): void;
  /** Reads another `ui://` resource on the widget's server. */
  readResource(uri: string): Promise<unknown>;
  /** The widget asked for this content height. */
  resize(height: number): void;
  /** The widget's content uses this width. */
  fitWidth(width: number): void;
}

export interface BridgeOptions {
  presentation: McpUiPresentation;
  html: string;
  theme: 'light' | 'dark';
  locale: string;
  handlers: BridgeHandlers;
  /** The proxy frame's window; messages from anything else are ignored. */
  source: () => Window | null | undefined;
  now?: () => number;
}

interface RpcMessage {
  jsonrpc: '2.0';
  id?: string | number;
  method?: string;
  params?: Record<string, unknown>;
}

class RpcError extends Error {
  constructor(
    readonly code: number,
    message: string
  ) {
    super(message);
  }
}

/** Whether `data` is a JSON-RPC 2.0 request or notification this host reads. */
export function readRpcMessage(data: unknown): RpcMessage | null {
  if (!data || typeof data !== 'object' || Array.isArray(data)) return null;
  const message = data as Record<string, unknown>;
  if (message.jsonrpc !== '2.0') return null;
  if (typeof message.method !== 'string' || message.method.length === 0) return null;
  if (message.method.length > MAX_METHOD_LENGTH) return null;
  if (
    'id' in message &&
    message.id !== undefined &&
    typeof message.id !== 'string' &&
    typeof message.id !== 'number'
  ) {
    return null;
  }
  if (
    message.params !== undefined &&
    (message.params === null || typeof message.params !== 'object' || Array.isArray(message.params))
  ) {
    return null;
  }
  return message as unknown as RpcMessage;
}

function textOf(content: unknown): string {
  if (!Array.isArray(content)) return '';
  return content
    .filter(
      (block): block is { type: 'text'; text: string } =>
        !!block &&
        typeof block === 'object' &&
        (block as { type?: unknown }).type === 'text' &&
        typeof (block as { text?: unknown }).text === 'string'
    )
    .map(block => block.text)
    .join('\n');
}

export class McpAppBridge {
  private readonly options: BridgeOptions;
  private presentation: McpUiPresentation;
  private delivered = false;
  private initialized = false;
  private lastOpenAt = Number.NEGATIVE_INFINITY;

  constructor(options: BridgeOptions) {
    this.options = options;
    this.presentation = options.presentation;
  }

  private get flavor() {
    return this.presentation.flavor;
  }

  /** A newer call of the same widget; an initialized widget receives its data. */
  updatePresentation(presentation: McpUiPresentation): void {
    const previous = this.presentation;
    this.presentation = presentation;
    if (!this.initialized) return;
    if (
      previous.tool_input === presentation.tool_input &&
      previous.structured_content === presentation.structured_content &&
      previous.result_meta === presentation.result_meta
    ) {
      return;
    }
    log('sending updated tool data');
    this.postToolData();
  }

  private post(message: Record<string, unknown>): void {
    const target = this.options.source();
    if (!target) return;
    target.postMessage({ jsonrpc: '2.0', ...message }, '*');
  }

  /** Feeds one `message` event; returns whether it was accepted. */
  handleMessage(event: MessageEvent): boolean {
    const source = this.options.source();
    if (!source || event.source !== source) return false;
    const message = readRpcMessage(event.data);
    if (!message) {
      log('dropped a malformed message');
      return false;
    }
    void this.dispatch(message);
    return true;
  }

  private async dispatch(message: RpcMessage): Promise<void> {
    const { method = '', id } = message;
    const params = message.params ?? {};
    if (id === undefined) {
      this.onNotification(method, params);
      return;
    }
    try {
      const result = await this.onRequest(method, params);
      this.post({ id, result });
    } catch (error) {
      const code = error instanceof RpcError ? error.code : RPC_INTERNAL_ERROR;
      const text = error instanceof Error ? error.message : 'Request failed';
      log('request %s failed: %s', method, text);
      this.post({ id, error: { code, message: text } });
    }
  }

  private onNotification(method: string, params: Record<string, unknown>): void {
    switch (method) {
      case 'ui/notifications/sandbox-proxy-ready':
        this.deliverDocument();
        return;
      case 'ui/notifications/initialized':
        this.sendToolData();
        return;
      case 'ui/notifications/size-changed': {
        if (params.fit === true) {
          const width = Number(params.width);
          if (Number.isFinite(width) && width > 0) this.options.handlers.fitWidth(Math.ceil(width));
          return;
        }
        const height = Number(params.height);
        if (Number.isFinite(height) && height > 0) {
          this.options.handlers.resize(
            Math.min(MAX_FRAME_HEIGHT, Math.max(MIN_FRAME_HEIGHT, Math.ceil(height)))
          );
        }
        return;
      }
      default:
        return;
    }
  }

  private deliverDocument(): void {
    if (this.delivered) return;
    this.delivered = true;
    const withAffordance = injectScript(
      injectScript(this.options.html, SCROLL_AFFORDANCE_SOURCE),
      frameFitSource(this.options.theme)
    );
    const html =
      this.flavor === 'apps_sdk'
        ? injectScript(withAffordance, APPS_SDK_SHIM_SOURCE)
        : withAffordance;
    log('delivering widget document (%s)', this.flavor);
    this.post({ method: 'ui/notifications/sandbox-resource-ready', params: { html } });
  }

  private sendToolData(): void {
    if (this.initialized) return;
    this.initialized = true;
    this.postToolData();
  }

  private postToolData(): void {
    const { presentation } = this;
    this.post({
      method: 'ui/notifications/tool-input',
      params: { arguments: presentation.tool_input ?? {} },
    });
    const result: Record<string, unknown> = { content: [] };
    if (presentation.structured_content !== undefined) {
      result.structuredContent = presentation.structured_content;
    }
    if (presentation.result_meta !== undefined) result._meta = presentation.result_meta;
    this.post({ method: 'ui/notifications/tool-result', params: result });
  }

  private async onRequest(method: string, params: Record<string, unknown>): Promise<unknown> {
    const { handlers } = this.options;
    switch (method) {
      case 'ui/initialize':
        return {
          protocolVersion:
            typeof params.protocolVersion === 'string' ? params.protocolVersion : PROTOCOL_VERSION,
          hostInfo: { name: 'OpenHuman', version: '1' },
          hostCapabilities: {
            openLinks: {},
            ...(this.flavor === 'host_inline' ? {} : { serverTools: {}, serverResources: {} }),
          },
          hostContext: {
            theme: this.options.theme,
            displayMode: 'inline',
            availableDisplayModes: ['inline'],
            locale: this.options.locale,
            platform: 'desktop',
          },
        };
      case 'ping':
        return {};
      case 'ui/request-display-mode':
        return { mode: 'inline' };
      case 'tools/call': {
        if (this.flavor === 'host_inline') {
          throw new RpcError(RPC_METHOD_NOT_FOUND, 'Tool calls are not available to this view');
        }
        const name = params.name;
        const args = params.arguments ?? {};
        if (typeof name !== 'string' || !name || typeof args !== 'object' || Array.isArray(args)) {
          throw new RpcError(RPC_INVALID_PARAMS, 'tools/call needs a name and an arguments object');
        }
        return handlers.callTool(name, args as Record<string, unknown>);
      }
      case 'ui/open-link': {
        const url = typeof params.url === 'string' ? params.url.trim() : '';
        const kind = classifyHref(url);
        if (kind !== 'external' && kind !== 'handoff') {
          throw new RpcError(RPC_DENIED, 'This link cannot be opened');
        }
        if (kind === 'handoff') {
          handlers.handOff(url);
          return {};
        }
        const now = (this.options.now ?? Date.now)();
        if (now - this.lastOpenAt < OPEN_LINK_INTERVAL_MS) {
          throw new RpcError(RPC_DENIED, 'Too many links opened');
        }
        this.lastOpenAt = now;
        await handlers.openExternal(url);
        return {};
      }
      case 'ui/message': {
        const text = textOf(params.content).trim();
        if (!text) throw new RpcError(RPC_INVALID_PARAMS, 'ui/message needs text content');
        handlers.prefillMessage(text);
        return {};
      }
      case 'resources/read': {
        const uri = typeof params.uri === 'string' ? params.uri : '';
        if (this.flavor === 'host_inline' || !/^ui:\/\/./i.test(uri)) {
          throw new RpcError(RPC_DENIED, 'Only ui:// resources on this server can be read');
        }
        return handlers.readResource(uri);
      }
      default:
        throw new RpcError(RPC_METHOD_NOT_FOUND, `Unsupported method ${method}`);
    }
  }
}
