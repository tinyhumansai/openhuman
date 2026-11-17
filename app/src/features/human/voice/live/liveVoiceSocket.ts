/**
 * Transport for one live voice session: the core's `/ws/live-voice` WebSocket.
 *
 * Same host/port as the core RPC (`getCoreHttpBaseUrl`), authenticated like the
 * dictation socket — the core bearer rides `?token=` because a browser
 * WebSocket cannot set an `Authorization` header. The URL therefore carries a
 * credential and is never logged.
 *
 * Wire contract (client → core): a JSON `start` frame first, then binary PCM16LE
 * 16 kHz mic frames and JSON `text` / `interrupt` / `stop` control frames.
 * Core → client: binary PCM16LE agent speech at `ready.output_sample_rate`, and
 * JSON events discriminated by `type` (see {@link LiveVoiceEvent}).
 */
import createDebug from 'debug';

import {
  getCoreHttpBaseUrl,
  getCoreRpcToken,
  resolveCoreSocketEndpoint,
} from '../../../../services/coreRpcClient';

const log = createDebug('app:human:live-voice');

export const LIVE_VOICE_WS_PATH = '/ws/live-voice';

export interface LiveVoiceStartFrame {
  type: 'start';
  provider: string | null;
  thread_id: string | null;
  input_sample_rate: number;
  /** Socket.IO client id of this window, so approvals route back here. */
  client_id?: string;
}

export type LiveVoiceEvent =
  | {
      type: 'ready';
      session_id: string;
      provider: string;
      output_sample_rate: number;
      thread_id: string | null;
    }
  | { type: 'transcript'; role: 'user' | 'agent'; text: string; final: boolean }
  | { type: 'tool_started'; call_id: string; name: string }
  | { type: 'tool_finished'; call_id: string; name: string; ok: boolean; cancelled: boolean }
  | { type: 'interrupted' }
  | { type: 'turn_complete' }
  | { type: 'error'; code: string; message: string; fatal: boolean }
  | { type: 'closed'; reason: string };

const KNOWN_EVENTS = new Set<LiveVoiceEvent['type']>([
  'ready',
  'transcript',
  'tool_started',
  'tool_finished',
  'interrupted',
  'turn_complete',
  'error',
  'closed',
]);

/** Parse one JSON text frame; `null` for anything that is not a known event. */
export function parseLiveVoiceEvent(raw: string): LiveVoiceEvent | null {
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!value || typeof value !== 'object') return null;
  const type = (value as { type?: unknown }).type;
  if (typeof type !== 'string' || !KNOWN_EVENTS.has(type as LiveVoiceEvent['type'])) return null;
  return value as LiveVoiceEvent;
}

/** Turn the core's HTTP base URL into the authenticated live-voice WS URL. */
export function buildLiveVoiceUrl(httpBaseUrl: string, token: string | null): string {
  const url = new URL(httpBaseUrl);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  url.pathname = LIVE_VOICE_WS_PATH;
  url.search = '';
  url.hash = '';
  if (token) url.searchParams.set('token', token);
  return url.toString();
}

/** Resolve the live-voice URL for the running core. Do not log the result. */
export async function resolveLiveVoiceUrl(): Promise<string> {
  const [base, token] = await Promise.all([getCoreHttpBaseUrl(), getCoreRpcToken()]);
  const endpoint = await resolveCoreSocketEndpoint(base);
  if (!endpoint.liveVoicePath) return buildLiveVoiceUrl(base, token);
  const url = new URL(endpoint.baseUrl);
  url.protocol = 'ws:';
  url.pathname = endpoint.liveVoicePath;
  url.search = '';
  url.hash = '';
  if (token) url.searchParams.set('token', token);
  return url.toString();
}

export interface LiveVoiceSocketHandlers {
  onOpen?: () => void;
  onEvent: (event: LiveVoiceEvent) => void;
  onAudio: (pcm: ArrayBuffer) => void;
  /** Socket closed — by either side. `clean` is false for network drops. */
  onClose: (info: { code: number; reason: string; clean: boolean }) => void;
}

/**
 * One WebSocket for one session. Sends the `start` frame as soon as it opens;
 * every send is a no-op once the socket is no longer open.
 */
export class LiveVoiceSocket {
  private ws: WebSocket;
  private closedByUs = false;

  constructor(url: string, start: LiveVoiceStartFrame, handlers: LiveVoiceSocketHandlers) {
    this.ws = new WebSocket(url);
    this.ws.binaryType = 'arraybuffer';
    this.ws.onopen = () => {
      log('[socket] open — sending start provider=%s', start.provider ?? 'default');
      this.ws.send(JSON.stringify(start));
      handlers.onOpen?.();
    };
    this.ws.onmessage = (message: MessageEvent) => {
      const { data } = message;
      if (data instanceof ArrayBuffer) {
        handlers.onAudio(data);
        return;
      }
      if (typeof data === 'string') {
        const event = parseLiveVoiceEvent(data);
        if (event) handlers.onEvent(event);
        else log('[socket] dropped unrecognised frame');
      }
    };
    this.ws.onerror = () => {
      log('[socket] error');
    };
    this.ws.onclose = (event: CloseEvent) => {
      log('[socket] closed code=%d by_us=%s', event.code, this.closedByUs);
      handlers.onClose({
        code: event.code,
        reason: event.reason,
        clean: this.closedByUs || event.wasClean,
      });
    };
  }

  get isOpen(): boolean {
    return this.ws.readyState === WebSocket.OPEN;
  }

  sendAudio(frame: ArrayBuffer): void {
    if (this.isOpen) this.ws.send(frame);
  }

  sendText(text: string): void {
    this.sendJson({ type: 'text', text });
  }

  interrupt(): void {
    this.sendJson({ type: 'interrupt' });
  }

  /** Ask the core to end the session, then close the socket. */
  stop(): void {
    this.sendJson({ type: 'stop' });
    this.close();
  }

  close(): void {
    if (this.closedByUs) return;
    this.closedByUs = true;
    if (this.ws.readyState === WebSocket.CONNECTING || this.ws.readyState === WebSocket.OPEN) {
      this.ws.close(1000, 'client stop');
    }
  }

  private sendJson(frame: Record<string, unknown>): void {
    if (this.isOpen) this.ws.send(JSON.stringify(frame));
  }
}
