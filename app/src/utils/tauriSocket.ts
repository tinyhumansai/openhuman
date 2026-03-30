/**
 * Tauri Socket Integration
 *
 * This module bridges the frontend with the Rust-native Socket.io client.
 *
 * In Tauri mode, the Socket.io connection lives in Rust (persistent,
 * survives app backgrounding). The frontend communicates via:
 *
 * - Tauri commands: runtime_socket_connect, runtime_socket_disconnect, etc.
 * - Tauri events: runtime:socket-state-changed, server:event
 *
 * Legacy bridge (for backwards compatibility during migration):
 * - socket:should_connect / socket:should_disconnect
 * - report_socket_connected / disconnected / error
 */
import { isTauri as coreIsTauri, invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';

import { syncToolsToBackend } from '../lib/skills/sync';
import { callCoreRpc } from '../services/coreRpcClient';
import { daemonHealthService } from '../services/daemonHealthService';
import { store } from '../store';
import { upsertChannelConnection } from '../store/channelConnectionsSlice';
import { setSocketIdForUser, setStatusForUser } from '../store/socketSlice';
import type { ChannelAuthMode, ChannelConnectionStatus, ChannelType } from '../types/channels';
import { BACKEND_URL } from './config';

let runtimeSocketCommandsAvailable = true;

function isCommandNotFoundError(error: unknown): boolean {
  const message =
    typeof error === 'string'
      ? error
      : error instanceof Error
        ? error.message
        : String(error ?? '');
  const normalized = message.toLowerCase();
  return normalized.includes('command') && normalized.includes('not found');
}

function handleRuntimeSocketInvokeError(context: string, error: unknown): void {
  if (isCommandNotFoundError(error)) {
    runtimeSocketCommandsAvailable = false;
    console.warn(
      `[TauriSocket] ${context} unavailable: runtime socket commands are not registered in this build`
    );
    return;
  }
  console.error(`[TauriSocket] Failed to ${context}:`, error);
}

// Check if we're running in Tauri
export const isTauri = (): boolean => {
  const isTauriEnv = coreIsTauri();
  const windowTauri = typeof window !== 'undefined' ? !!window.__TAURI__ : 'undefined';
  const userAgent = typeof navigator !== 'undefined' ? navigator.userAgent : 'undefined';
  console.log(
    '[TauriSocket] isTauri() check:',
    isTauriEnv,
    'window.__TAURI__:',
    windowTauri,
    'userAgent:',
    userAgent
  );
  return isTauriEnv;
};

// ---------------------------------------------------------------------------
// Rust-native socket commands (Phase 2)
// ---------------------------------------------------------------------------

/**
 * Connect the Rust-native Socket.io client to the backend.
 * The Rust client handles MCP protocol directly.
 */
export async function connectRustSocket(token: string): Promise<void> {
  if (!isTauri()) return;
  if (!runtimeSocketCommandsAvailable) return;

  try {
    console.log('[TauriSocket] Connecting Rust socket to', BACKEND_URL);
    await invoke('runtime_socket_connect', { token, url: BACKEND_URL });
    console.log('[TauriSocket] Rust socket connect call succeeded');
  } catch (error) {
    handleRuntimeSocketInvokeError('connect Rust socket', error);
    // Ensure Redux status reflects the failure
    const uid = getSocketUserId();
    store.dispatch(setStatusForUser({ userId: uid, status: 'disconnected' }));
  }
}

/**
 * Disconnect the Rust-native Socket.io client.
 */
export async function disconnectRustSocket(): Promise<void> {
  if (!isTauri()) return;
  if (!runtimeSocketCommandsAvailable) return;

  try {
    await invoke('runtime_socket_disconnect');
    console.log('[TauriSocket] Rust socket disconnected');
  } catch (error) {
    handleRuntimeSocketInvokeError('disconnect Rust socket', error);
  }
}

/**
 * Emit an event through the Rust socket to the server.
 * Use this when the frontend needs to send events in Tauri mode.
 */
export async function emitViaRustSocket(event: string, data?: unknown): Promise<void> {
  if (!isTauri()) return;
  if (!runtimeSocketCommandsAvailable) return;

  try {
    await invoke('runtime_socket_emit', { event, data: data ?? null });
  } catch (error) {
    handleRuntimeSocketInvokeError('emit via Rust socket', error);
  }
}

/**
 * Get the current Rust socket state.
 */
export async function getRustSocketState(): Promise<{
  status: string;
  socket_id: string | null;
} | null> {
  if (!isTauri()) return null;
  if (!runtimeSocketCommandsAvailable) return null;

  try {
    return await invoke('runtime_socket_state');
  } catch (error) {
    handleRuntimeSocketInvokeError('get Rust socket state', error);
    return null;
  }
}

// ---------------------------------------------------------------------------
// Tauri event listeners
// ---------------------------------------------------------------------------

let unlistenConnect: UnlistenFn | null = null;
let unlistenDisconnect: UnlistenFn | null = null;
let unlistenSocketState: UnlistenFn | null = null;
let unlistenServerEvent: UnlistenFn | null = null;
let unlistenDaemonHealth: UnlistenFn | null = null;

interface ChannelConnectionUpdatedEvent {
  channel: ChannelType;
  authMode: ChannelAuthMode;
  status: ChannelConnectionStatus;
  lastError?: string;
  capabilities?: string[];
}

function isChannelConnectionUpdatePayload(value: unknown): value is ChannelConnectionUpdatedEvent {
  if (!value || typeof value !== 'object') return false;
  const obj = value as Record<string, unknown>;
  const channel = obj.channel;
  const authMode = obj.authMode;
  const status = obj.status;
  return (
    (channel === 'telegram' || channel === 'discord') &&
    (authMode === 'managed_dm' ||
      authMode === 'oauth' ||
      authMode === 'bot_token' ||
      authMode === 'api_key') &&
    (status === 'connected' ||
      status === 'connecting' ||
      status === 'disconnected' ||
      status === 'error')
  );
}

function getSocketUserId(): string {
  const token = store.getState().auth.token;
  if (!token) return '__pending__';

  try {
    const parts = token.split('.');
    if (parts.length !== 3) return '__pending__';
    const payloadBase64 = parts[1].replace(/-/g, '+').replace(/_/g, '/');
    const payloadJson = atob(payloadBase64);
    const payload = JSON.parse(payloadJson);
    return payload.tgUserId || payload.userId || payload.sub || '__pending__';
  } catch {
    return '__pending__';
  }
}

/**
 * Setup Tauri socket event listeners.
 * This should be called once when the app starts in Tauri mode.
 */
export async function setupTauriSocketListeners(): Promise<void> {
  console.log('[TauriSocket] setupTauriSocketListeners() called');
  if (!isTauri()) {
    console.log('[TauriSocket] Not in Tauri environment, returning early');
    return;
  }
  console.log('[TauriSocket] In Tauri environment, proceeding with listener setup');

  try {
    console.log('[TauriSocket] Starting listener setup sequence');
    // Listen for Rust socket state changes (Phase 2 — primary)
    console.log('[TauriSocket] Setting up runtime:socket-state-changed listener');
    unlistenSocketState = await listen<{ status: string; socket_id: string | null }>(
      'runtime:socket-state-changed',
      event => {
        const { status, socket_id } = event.payload;
        const uid = getSocketUserId();

        console.log('[TauriSocket] Rust socket state:', status, socket_id);

        // Map Rust status to Redux SocketConnectionStatus
        // Redux only supports: 'connected' | 'disconnected' | 'connecting'
        type ReduxStatus = 'connected' | 'disconnected' | 'connecting';
        const statusMap: Record<string, ReduxStatus> = {
          connected: 'connected',
          connecting: 'connecting',
          disconnected: 'disconnected',
          reconnecting: 'connecting', // map to 'connecting'
          error: 'disconnected', // map to 'disconnected'
        };

        const mappedStatus: ReduxStatus = statusMap[status] || 'disconnected';
        store.dispatch(setStatusForUser({ userId: uid, status: mappedStatus }));
        store.dispatch(setSocketIdForUser({ userId: uid, socketId: socket_id ?? null }));

        if (mappedStatus === 'connected') {
          syncToolsToBackend();
        }
      }
    );
    console.log('[TauriSocket] runtime:socket-state-changed listener setup complete');

    // Listen for forwarded server events
    console.log('[TauriSocket] Setting up server:event listener');
    unlistenServerEvent = await listen<{ event: string; data: unknown }>('server:event', event => {
      const { event: eventName, data } = event.payload;
      console.log('[TauriSocket] Server event:', eventName, data);
      if (eventName === 'channel:connection-updated' && isChannelConnectionUpdatePayload(data)) {
        store.dispatch(
          upsertChannelConnection({
            channel: data.channel,
            authMode: data.authMode,
            patch: {
              status: data.status,
              lastError: data.lastError,
              capabilities: data.capabilities ?? [],
            },
          })
        );
      }
    });
    console.log('[TauriSocket] server:event listener setup complete');

    // Legacy: Listen for connect requests from Rust (backwards compat)
    console.log('[TauriSocket] Setting up legacy socket:should_connect listener');
    unlistenConnect = await listen<{ backendUrl: string; token: string }>(
      'socket:should_connect',
      async event => {
        console.log('[TauriSocket] Legacy connect request (ignored — using Rust socket)');
        // No-op: Rust socket handles connection now
        void event;
      }
    );
    console.log('[TauriSocket] socket:should_connect listener setup complete');

    // Legacy: Listen for disconnect requests from Rust
    console.log('[TauriSocket] Setting up legacy socket:should_disconnect listener');
    unlistenDisconnect = await listen('socket:should_disconnect', async () => {
      console.log('[TauriSocket] Legacy disconnect request (ignored — using Rust socket)');
      // No-op: Rust socket handles disconnection now
    });
    console.log('[TauriSocket] socket:should_disconnect listener setup complete');

    // Setup daemon health monitoring
    console.log('[TauriSocket] About to setup daemon health listener');
    unlistenDaemonHealth = await daemonHealthService.setupHealthListener();
    console.log('[TauriSocket] Daemon health listener setup result:', unlistenDaemonHealth);

    console.log('[TauriSocket] Event listeners setup complete');
  } catch (error) {
    console.error('[TauriSocket] Failed to setup listeners:', error);
  }
}

/**
 * Cleanup Tauri socket event listeners.
 */
export function cleanupTauriSocketListeners(): void {
  if (unlistenConnect) {
    unlistenConnect();
    unlistenConnect = null;
  }
  if (unlistenDisconnect) {
    unlistenDisconnect();
    unlistenDisconnect = null;
  }
  if (unlistenSocketState) {
    unlistenSocketState();
    unlistenSocketState = null;
  }
  if (unlistenServerEvent) {
    unlistenServerEvent();
    unlistenServerEvent = null;
  }
  if (unlistenDaemonHealth) {
    unlistenDaemonHealth();
    unlistenDaemonHealth = null;
  }

  // Cleanup daemon health service
  daemonHealthService.cleanup();
}

// ---------------------------------------------------------------------------
// Legacy reporting functions (kept for backwards compatibility)
// ---------------------------------------------------------------------------

/**
 * Report socket connected status to Rust (legacy — used in web mode).
 */
export async function reportSocketConnected(socketId?: string): Promise<void> {
  void socketId;
}

/**
 * Report socket disconnected status to Rust (legacy — used in web mode).
 */
export async function reportSocketDisconnected(): Promise<void> {
  // Legacy no-op: socket status is now sourced from runtime:socket-state-changed events.
}

/**
 * Report socket error to Rust (legacy — used in web mode).
 */
export async function reportSocketError(error: string): Promise<void> {
  console.warn('[TauriSocket] Legacy reportSocketError no-op:', error);
}

/**
 * Update socket status in Rust (legacy — used in web mode).
 */
export async function updateSocketStatus(
  status: 'connected' | 'connecting' | 'disconnected' | 'reconnecting' | 'error',
  socketId?: string
): Promise<void> {
  void status;
  void socketId;
}

/**
 * Get session token from Rust secure storage.
 */
export async function getSecureToken(): Promise<string | null> {
  if (!isTauri()) return null;

  try {
    const response = await callCoreRpc<{ result: { token: string | null } }>({
      method: 'openhuman.auth.get_session_token',
    });
    return response.result.token;
  } catch (error) {
    console.error('[TauriSocket] Failed to get secure token:', error);
    return null;
  }
}

/**
 * Check if user is authenticated (from Rust).
 */
export async function isAuthenticatedFromRust(): Promise<boolean> {
  if (!isTauri()) return false;

  try {
    const response = await callCoreRpc<{ result: { isAuthenticated: boolean } }>({
      method: 'openhuman.auth.get_state',
    });
    return response.result.isAuthenticated;
  } catch (error) {
    console.error('[TauriSocket] Failed to check auth:', error);
    return false;
  }
}
