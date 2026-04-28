import { isTauri as coreIsTauri, invoke } from '@tauri-apps/api/core';
import debug from 'debug';
import { io, Socket } from 'socket.io-client';

import { getCoreStateSnapshot } from '../lib/coreState/store';
import { SocketIOMCPTransportImpl } from '../lib/mcp';
import { store } from '../store';
import { upsertChannelConnection } from '../store/channelConnectionsSlice';
import { resetForUser, setSocketIdForUser, setStatusForUser } from '../store/socketSlice';
import type { ChannelAuthMode, ChannelConnectionStatus, ChannelType } from '../types/channels';
import { CORE_RPC_URL, IS_DEV } from '../utils/config';
import { createSafeLogData, sanitizeError } from '../utils/sanitize';

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

async function resolveCoreSocketBaseUrl(): Promise<string> {
  if (!coreIsTauri()) {
    return coreSocketBaseFromRpcUrl(CORE_RPC_URL);
  }

  try {
    const rpcUrl = await invoke<string>('core_rpc_url');
    return coreSocketBaseFromRpcUrl(String(rpcUrl || CORE_RPC_URL));
  } catch {
    return coreSocketBaseFromRpcUrl(CORE_RPC_URL);
  }
}

interface JwtPayload {
  tgUserId?: string;
  userId?: string;
  sub?: string;
}

interface ChannelConnectionUpdatedEvent {
  channel: ChannelType;
  authMode: ChannelAuthMode;
  status: ChannelConnectionStatus;
  lastError?: string;
  capabilities?: string[];
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

function getSocketUserId(): string {
  const token = getCoreStateSnapshot().snapshot.sessionToken;
  if (!token) return '__pending__';

  try {
    const parts = token.split('.');
    if (parts.length !== 3) return '__pending__';

    const payloadBase64 = parts[1].replace(/-/g, '+').replace(/_/g, '/');
    const payloadJson = atob(payloadBase64);
    const payload = JSON.parse(payloadJson) as JwtPayload;

    const id = payload.tgUserId || payload.userId || payload.sub;
    return id || '__pending__';
  } catch {
    return '__pending__';
  }
}

class SocketService {
  private socket: Socket | null = null;
  private token: string | null = null;
  private mcpTransport: SocketIOMCPTransportImpl | null = null;
  private pendingListeners: Array<{ event: string; callback: (...args: unknown[]) => void }> = [];
  // Maps original caller callbacks → wrapped callbacks so off() can locate the
  // exact function reference that was registered with socket.io, scoped by event.
  private listenerMap = new Map<
    string,
    Map<(...args: unknown[]) => void, (...args: unknown[]) => void>
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
      }
    }

    this.token = token;
    const uid = getSocketUserId();
    store.dispatch(setStatusForUser({ userId: uid, status: 'connecting' }));

    const backendUrl = await resolveCoreSocketBaseUrl();
    socketLog('Connecting to core socket', { userId: uid, backendUrl });

    // Ensure we're not connecting to the wrong URL
    if (backendUrl.includes('localhost:1420') || backendUrl.includes(':1420')) {
      return;
    }

    const socketOptions = {
      auth: { token },
      path: '/socket.io/',
      transports: ['websocket', 'polling'] as ('websocket' | 'polling')[],
      reconnection: true,
      reconnectionDelay: 1000,
      reconnectionAttempts: 5,
      forceNew: true,
      timeout: 2000,
      upgrade: true,
      query: {},
    };

    this.socket = io(backendUrl, socketOptions);

    // Flush any listeners that were registered before the socket existed.
    if (this.pendingListeners.length > 0) {
      socketLog('Flushing pending listeners', { count: this.pendingListeners.length });
      for (const { event, callback } of this.pendingListeners) {
        this.socket.on(event, callback);
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
    });

    this.socket.on('connect_error', (error: Error) => {
      const uid = getSocketUserId();
      socketError('Connection error', { userId: uid, error: sanitizeError(error) });
      store.dispatch(setStatusForUser({ userId: uid, status: 'disconnected' }));
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

    this.socket.connect();
  }

  /**
   * Disconnect from the socket server
   */
  disconnect(): void {
    if (this.socket) {
      const uid = getSocketUserId();
      socketLog('Disconnecting', { userId: uid });
      this.socket.disconnect();
      this.socket = null;
      this.token = null;
      this.mcpTransport = null;
      this.listenerMap.clear();
      store.dispatch(resetForUser({ userId: uid }));
    }
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
    byEvent.set(callback, wrappedCallback);
    this.listenerMap.set(event, byEvent);
    if (this.socket) {
      this.socket.on(event, wrappedCallback);
    } else {
      socketLog('Socket not ready, queuing listener', { event });
      this.pendingListeners.push({ event, callback: wrappedCallback });
    }
  }

  /**
   * Remove an event listener
   */
  off(event: string, callback?: (...args: unknown[]) => void): void {
    if (callback) {
      const byEvent = this.listenerMap.get(event);
      const wrapped = byEvent?.get(callback) ?? callback;
      const hadWrapped = byEvent?.has(callback) ?? false;
      byEvent?.delete(callback);
      if (byEvent && byEvent.size === 0) {
        this.listenerMap.delete(event);
      }
      socketLog('Removing listener', { event, hadWrappedVersion: hadWrapped });
      if (this.socket) {
        this.socket.off(event, wrapped);
      }
      // Also remove from the pending queue in case the socket isn't up yet.
      this.pendingListeners = this.pendingListeners.filter(
        p => !(p.event === event && p.callback === wrapped)
      );
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
      callback(...args);
    };
    if (this.socket) {
      this.socket.once(event, wrappedCallback);
    } else {
      socketLog('Socket not ready, queuing once listener', { event });
      this.pendingListeners.push({ event, callback: wrappedCallback });
    }
  }
}

export const socketService = new SocketService();
