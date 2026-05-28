import debug from 'debug';
import { type Socket } from 'socket.io-client';

import { getCoreStateSnapshot } from '../lib/coreState/store';
import { SocketIOMCPTransportImpl } from '../lib/mcp';
import { store } from '../store';
import { upsertChannelConnection } from '../store/channelConnectionsSlice';
import { type CompanionStateChangedEvent, setCompanionState } from '../store/companionSlice';
import { setBackend } from '../store/connectivitySlice';
import { resetForUser, setSocketIdForUser, setStatusForUser } from '../store/socketSlice';
import type { ChannelAuthMode, ChannelConnectionStatus, ChannelType } from '../types/channels';
import { IS_DEV } from '../utils/config';
import { createSafeLogData, sanitizeError } from '../utils/sanitize';
import { getCoreRpcToken, getCoreRpcUrl } from './coreRpcClient';
import { createCoreSocket } from './coreSocket';

// Socket service logger using debug package
// Enable logging by setting DEBUG=socket* in environment or localStorage
const socketLog = debug('socket');
const socketWarn = debug('socket:warn');
const socketError = debug('socket:error');

// Enable socket logging in development by default
if (IS_DEV) {
  debug.enable('socket*');
}

function coreSocketBaseFromRpcUrl(rpcUrl: string): string {
  const trimmed = rpcUrl.trim().replace(/\/+$/, '');
  return trimmed.endsWith('/rpc') ? trimmed.slice(0, -4) : trimmed;
}

/**
 * Resolve the Socket.IO base URL from the user's stored RPC URL preference.
 * Delegates to getCoreRpcUrl() so the stored preference (set on the Welcome
 * screen) is always honoured — previously this called invoke('core_rpc_url')
 * directly, which ignored the user's stored override.
 */
async function resolveCoreSocketBaseUrl(): Promise<string> {
  const rpcUrl = await getCoreRpcUrl();
  return coreSocketBaseFromRpcUrl(rpcUrl);
}

interface ChannelConnectionUpdatedEvent {
  channel: ChannelType;
  authMode: ChannelAuthMode;
  status: ChannelConnectionStatus;
  lastError?: string;
  capabilities?: string[];
}

interface PendingSocketListener {
  event: string;
  callback: (...args: unknown[]) => void;
  once: boolean;
}

function normalizeChannelConnectionUpdatePayload(
  value: unknown
): ChannelConnectionUpdatedEvent | null {
  if (!value || typeof value !== 'object') return null;

  const obj = value as Record<string, unknown>;
  const channel = obj.channel;
  const authMode = obj.authMode ?? obj.auth_mode;
  const status = obj.status;
  const lastError = obj.lastError ?? obj.last_error;
  const capabilities = obj.capabilities;

  const isKnownChannel = channel === 'telegram' || channel === 'discord' || channel === 'web';
  const isKnownAuthMode =
    authMode === 'managed_dm' ||
    authMode === 'oauth' ||
    authMode === 'bot_token' ||
    authMode === 'api_key';
  const isKnownStatus =
    status === 'connected' ||
    status === 'connecting' ||
    status === 'disconnected' ||
    status === 'error';

  if (!isKnownChannel || !isKnownAuthMode || !isKnownStatus) {
    return null;
  }

  return {
    channel,
    authMode,
    status,
    lastError: typeof lastError === 'string' ? lastError : undefined,
    capabilities: Array.isArray(capabilities)
      ? capabilities.filter((item): item is string => typeof item === 'string')
      : undefined,
  };
}

const COMPANION_STATES: ReadonlySet<string> = new Set([
  'idle',
  'listening',
  'thinking',
  'speaking',
  'pointing',
  'error',
]);

export function parseCompanionStateChangedEvent(value: unknown): CompanionStateChangedEvent | null {
  if (!value || typeof value !== 'object') return null;
  const obj = value as Record<string, unknown>;
  if (typeof obj.session_id !== 'string') return null;
  if (typeof obj.state !== 'string' || !COMPANION_STATES.has(obj.state)) return null;

  const previous =
    typeof obj.previous_state === 'string' && COMPANION_STATES.has(obj.previous_state)
      ? (obj.previous_state as CompanionStateChangedEvent['previous_state'])
      : 'idle';
  const message = typeof obj.message === 'string' ? obj.message : undefined;

  return {
    session_id: obj.session_id,
    state: obj.state as CompanionStateChangedEvent['state'],
    previous_state: previous,
    message,
  };
}

function getSocketUserId(): string {
  return getCoreStateSnapshot().snapshot?.auth?.userId ?? '__pending__';
}

class SocketService {
  private socket: Socket | null = null;
  private token: string | null = null;
  private mcpTransport: SocketIOMCPTransportImpl | null = null;
  private pendingListeners: PendingSocketListener[] = [];
  // Maps original caller callbacks → wrapped callbacks so off() can locate the
  // exact function references that were registered with socket.io, scoped by event.
  private listenerMap = new Map<
    string,
    Map<(...args: unknown[]) => void, Set<(...args: unknown[]) => void>>
  >();

  /**
   * Connect to the socket server with authentication.
   */
  connect(token: string): void {
    void this.connectAsync(token);
  }

  private async connectAsync(token: string): Promise<void> {
    if (!token) return;

    // Don't connect if already connected with the same token
    if (this.socket?.connected && this.token === token) return;

    // Disconnect existing connection if token changed or socket exists
    if (this.socket) {
      if (this.token !== token) {
        this.disconnect();
      } else if (this.socket.connected) {
        return;
      } else if (!this.socket.disconnected) {
        // Socket is connecting, wait for it
        return;
      } else {
        // Stale disconnected socket instance for the same token.
        // Drop it so this connect attempt can create a fresh socket;
        // otherwise the async stale-invocation guard below (`|| this.socket`)
        // returns early and leaves connectivity stuck at "connecting".
        this.socket = null;
        this.mcpTransport = null;
      }
    }

    this.token = token;
    const uid = getSocketUserId();
    store.dispatch(setStatusForUser({ userId: uid, status: 'connecting' }));
    // Mirror backend Socket.IO state into the connectivity channel (#1527).
    store.dispatch(setBackend({ value: 'connecting' }));

    const backendUrl = await resolveCoreSocketBaseUrl();
    // If another `connect(token)` raced in while the URL was resolving,
    // a stale invocation will see `this.token` flipped to the newer JWT
    // (or a fresh socket already attached) and must bail before its
    // io(...) call stomps the newer connection. Same guard repeats
    // after the core-token resolve below.
    if (this.token !== token || this.socket) return;
    socketLog('Connecting to core socket', { userId: uid, backendUrl });

    // Ensure we're not connecting to the wrong URL (Vite dev HMR port guard).
    // Reset the backend channel before returning so it doesn't stay stuck at
    // 'connecting'. (addresses @coderabbitai on socketService.ts:154-163)
    if (backendUrl.includes('localhost:1420') || backendUrl.includes(':1420')) {
      store.dispatch(
        setBackend({ value: 'disconnected', error: 'dev-server URL guard — not a real backend' })
      );
      return;
    }

    // The local core's Socket.IO handshake validates the per-process bearer
    // exposed via `core_rpc_token` (Tauri IPC) / the cloud-mode picker. The
    // session JWT rides alongside on the `auth` payload as `session` so a
    // future handler can correlate the connection with the logged-in user.
    const coreToken = await getCoreRpcToken();
    if (this.token !== token || this.socket) return;

    this.socket = createCoreSocket(backendUrl, {
      coreToken,
      authExtras: { session: token },
      overrides: {
        // A remote / tunnelled core (e.g. ~0.8s RTT) needs the socket.io
        // handshake (~2.4s) to outlast the connect timeout — the prior 2s
        // tripped first, flapping the socket and dropping streamed/approval
        // events. 10s gives headroom while still surfacing a genuinely dead
        // core reasonably fast; keep retrying rather than giving up after 5.
        reconnectionDelay: 2000,
        reconnectionAttempts: Infinity,
        timeout: 10000,
        upgrade: true,
        query: {},
      },
    });

    // Flush any listeners that were registered before the socket existed.
    if (this.pendingListeners.length > 0) {
      socketLog('Flushing pending listeners', { count: this.pendingListeners.length });
      for (const { event, callback, once } of this.pendingListeners) {
        if (once) {
          this.socket.once(event, callback);
        } else {
          this.socket.on(event, callback);
        }
      }
      this.pendingListeners = [];
    }

    this.socket.onAny((event, ...args) => {
      const firstArg = args.length > 0 ? args[0] : undefined;
      socketLog(
        'Inbound event',
        createSafeLogData({ event, argsCount: args.length, hasData: args.length > 0 }, firstArg)
      );
    });

    // Initialize MCP transport for client→server MCP requests
    this.mcpTransport = new SocketIOMCPTransportImpl(this.socket);

    // Connection event handlers
    this.socket.on('connect', () => {
      const socketId = this.socket?.id || null;
      const uid = getSocketUserId();
      socketLog('Connected', { socketId, userId: uid });
      store.dispatch(setStatusForUser({ userId: uid, status: 'connected' }));
      store.dispatch(setSocketIdForUser({ userId: uid, socketId }));
      store.dispatch(setBackend({ value: 'connected' }));

      // Re-join the active thread's room so an in-flight turn's stream survives
      // this (re)connection. Chat events are delivered to both the client_id
      // room and a per-thread room (see socketio.rs `emit_web_channel_event`);
      // because a reconnect produces a NEW client_id, the new socket must
      // re-subscribe to the thread room to keep receiving the stream.
      const threadState = store.getState().thread;
      const activeThreadId = threadState?.selectedThreadId ?? threadState?.activeThreadId;
      if (activeThreadId) {
        this.socket?.emit('thread:subscribe', { thread_id: activeThreadId });
      }
    });

    this.socket.on('ready', () => {
      const uid = getSocketUserId();
      socketLog('Server ready - authentication successful', { userId: uid });
    });

    this.socket.on('error', (error: unknown) => {
      const uid = getSocketUserId();
      socketError('Server error', { userId: uid, error: sanitizeError(error) });
    });

    this.socket.on('disconnect', (reason: string) => {
      const uid = getSocketUserId();
      socketLog('Disconnected', { userId: uid, reason });
      store.dispatch(setStatusForUser({ userId: uid, status: 'disconnected' }));
      store.dispatch(setSocketIdForUser({ userId: uid, socketId: null }));
      store.dispatch(setBackend({ value: 'disconnected', error: reason }));
    });

    this.socket.on('connect_error', (error: Error) => {
      const uid = getSocketUserId();
      socketError('Connection error', { userId: uid, error: sanitizeError(error) });
      store.dispatch(setStatusForUser({ userId: uid, status: 'disconnected' }));
      store.dispatch(
        setBackend({
          value: 'disconnected',
          error: error instanceof Error ? error.message : String(error),
        })
      );
    });

    const handleChannelConnectionUpdated = (data: unknown) => {
      const payload = normalizeChannelConnectionUpdatePayload(data);
      if (!payload) return;

      store.dispatch(
        upsertChannelConnection({
          channel: payload.channel,
          authMode: payload.authMode,
          patch: {
            status: payload.status,
            lastError: payload.lastError,
            ...(payload.capabilities !== undefined && { capabilities: payload.capabilities }),
          },
        })
      );
    };

    this.socket.on('channel:connection-updated', handleChannelConnectionUpdated);
    this.socket.on('channel_connection_updated', handleChannelConnectionUpdated);

    // Core-side session expiry (401 from the OpenHuman backend or jsonrpc).
    // The server has already published SessionExpired on its event bus,
    // the credentials subscriber has cleared the JWT, and the scheduler
    // gate is flipped to signed-out. All the UI needs to do is mirror
    // that locally and route to onboarding. CoreStateProvider listens
    // for the window event below and calls its own `clearSession`.
    const handleSessionExpired = (data: unknown) => {
      const source =
        (data && typeof data === 'object' && 'source' in data && typeof data.source === 'string'
          ? data.source
          : undefined) ?? 'unknown';
      socketLog('Session expired notification received', { source });
      if (typeof window !== 'undefined') {
        window.dispatchEvent(new CustomEvent('openhuman:session-expired', { detail: { source } }));
      }
    };
    this.socket.on('auth:session_expired', handleSessionExpired);
    this.socket.on('auth_session_expired', handleSessionExpired);

    // MCP setup agent: server-side `request_secret` blocks until the
    // user submits a value. Dispatch a window event so a singleton React
    // dialog can render a native input and POST back via
    // openhuman.mcp_setup_submit_secret. Raw secret values never travel
    // through the socket — only the opaque ref + safe display fields.
    const handleSecretRequested = (data: unknown) => {
      const obj = data as Record<string, unknown> | null;
      if (!obj || typeof obj !== 'object') {
        socketWarn('mcp_setup:secret_requested dropped — invalid payload');
        return;
      }
      const refId = typeof obj.ref_id === 'string' ? obj.ref_id : null;
      const keyName = typeof obj.key_name === 'string' ? obj.key_name : null;
      const prompt = typeof obj.prompt === 'string' ? obj.prompt : '';
      if (!refId || !keyName) {
        socketWarn('mcp_setup:secret_requested missing ref_id or key_name');
        return;
      }
      socketLog('mcp_setup:secret_requested', { refId, keyName });
      if (typeof window !== 'undefined') {
        window.dispatchEvent(
          new CustomEvent('openhuman:mcp-setup-secret-requested', {
            detail: { refId, keyName, prompt },
          })
        );
      }
    };
    this.socket.on('mcp_setup:secret_requested', handleSecretRequested);
    this.socket.on('mcp_setup_secret_requested', handleSecretRequested);

    this.socket.on('channel:managed-dm-verified', data => {
      const obj = data as Record<string, unknown> | null;
      if (!obj || typeof obj !== 'object') return;
      const token = typeof obj.token === 'string' ? obj.token : undefined;
      const telegramUsername =
        typeof obj.telegramUsername === 'string' ? obj.telegramUsername : undefined;
      const chatId = typeof obj.chatId === 'number' ? obj.chatId : undefined;
      if (!token) return;

      socketLog('Managed DM verified', { tokenLength: token.length, telegramUsername, chatId });
      store.dispatch(
        upsertChannelConnection({
          channel: 'telegram',
          authMode: 'managed_dm',
          patch: { status: 'connected', lastError: undefined, capabilities: ['dm'] },
        })
      );
    });

    // Companion state change events — dispatch into the companion Redux slice
    // so settings panel and other UI can react to session lifecycle.
    this.socket.on('companion:state_changed', (data: unknown) => {
      const event = parseCompanionStateChangedEvent(data);
      if (!event) {
        socketWarn('companion:state_changed dropped — invalid payload shape');
        return;
      }
      socketLog('companion:state_changed → %s', event.state);
      store.dispatch(setCompanionState(event));
    });

    this.socket.connect();
  }

  /**
   * Disconnect from the socket server
   */
  disconnect(): void {
    const uid = getSocketUserId();
    if (this.socket) {
      socketLog('Disconnecting', { userId: uid });
      this.socket.disconnect();
      this.socket = null;
      store.dispatch(resetForUser({ userId: uid }));
    }
    this.token = null;
    this.mcpTransport = null;
    this.listenerMap.clear();
    this.pendingListeners = [];
  }

  /**
   * Get the current socket instance
   */
  getSocket(): Socket | null {
    return this.socket;
  }

  /**
   * Get the MCP transport for making client→server MCP requests
   */
  getMCPTransport(): SocketIOMCPTransportImpl | null {
    return this.mcpTransport;
  }

  /**
   * Check if socket is connected
   */
  isConnected(): boolean {
    return this.socket?.connected || false;
  }

  /**
   * Emit an event to the server
   */
  emit(event: string, data?: unknown): void {
    if (this.socket?.connected) {
      socketLog('Emitting event', createSafeLogData({ event }, data));
      this.socket.emit(event, data);
    } else {
      socketWarn('Cannot emit event - socket not connected', { event });
    }
  }

  /**
   * Listen to an event from the server
   */
  on(event: string, callback: (...args: unknown[]) => void): void {
    const wrappedCallback = (...args: unknown[]) => {
      socketLog('Received event', { event, argsCount: args.length, hasData: args.length > 0 });
      callback(...args);
    };
    // Track original→wrapped per event so the same callback can be used for
    // multiple events without collisions.
    const byEvent = this.listenerMap.get(event) ?? new Map();
    const wrappedSet = byEvent.get(callback) ?? new Set();
    wrappedSet.add(wrappedCallback);
    byEvent.set(callback, wrappedSet);
    this.listenerMap.set(event, byEvent);
    if (this.socket) {
      this.socket.on(event, wrappedCallback);
    } else {
      socketLog('Socket not ready, queuing listener', { event });
      this.pendingListeners.push({ event, callback: wrappedCallback, once: false });
    }
  }

  /**
   * Remove an event listener
   */
  off(event: string, callback?: (...args: unknown[]) => void): void {
    if (callback) {
      const byEvent = this.listenerMap.get(event);
      const wrappedSet = byEvent?.get(callback);
      const wrappedCallbacks =
        wrappedSet && wrappedSet.size > 0 ? Array.from(wrappedSet) : [callback];
      const hadWrapped = !!wrappedSet && wrappedSet.size > 0;
      byEvent?.delete(callback);
      if (byEvent && byEvent.size === 0) {
        this.listenerMap.delete(event);
      }
      socketLog('Removing listener', { event, hadWrappedVersion: hadWrapped });
      for (const wrapped of wrappedCallbacks) {
        if (this.socket) {
          this.socket.off(event, wrapped);
        }
        // Also remove from the pending queue in case the socket isn't up yet.
        this.pendingListeners = this.pendingListeners.filter(
          p => !(p.event === event && p.callback === wrapped)
        );
      }
    } else {
      socketLog('Removing all listeners for event', { event });
      this.socket?.off(event);
      this.pendingListeners = this.pendingListeners.filter(p => p.event !== event);
      this.listenerMap.delete(event);
    }
  }

  /**
   * Listen to an event once
   */
  once(event: string, callback: (...args: unknown[]) => void): void {
    const wrappedCallback = (...args: unknown[]) => {
      socketLog('Received event (once)', {
        event,
        argsCount: args.length,
        hasData: args.length > 0,
      });
      try {
        callback(...args);
      } finally {
        const byEvent = this.listenerMap.get(event);
        const wrappedSet = byEvent?.get(callback);
        wrappedSet?.delete(wrappedCallback);
        if (wrappedSet && wrappedSet.size === 0) {
          byEvent?.delete(callback);
        }
        if (byEvent && byEvent.size === 0) {
          this.listenerMap.delete(event);
        }
      }
    };
    const byEvent = this.listenerMap.get(event) ?? new Map();
    const wrappedSet = byEvent.get(callback) ?? new Set();
    wrappedSet.add(wrappedCallback);
    byEvent.set(callback, wrappedSet);
    this.listenerMap.set(event, byEvent);
    if (this.socket) {
      this.socket.once(event, wrappedCallback);
    } else {
      socketLog('Socket not ready, queuing once listener', { event });
      this.pendingListeners.push({ event, callback: wrappedCallback, once: true });
    }
  }
}

export const socketService = new SocketService();
