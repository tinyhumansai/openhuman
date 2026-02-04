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

import { store } from '../store';
import { setSocketIdForUser, setStatusForUser } from '../store/socketSlice';
import { BACKEND_URL } from './config';

// Check if we're running in Tauri
export const isTauri = (): boolean => {
  return coreIsTauri();
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

  try {
    await invoke('runtime_socket_connect', { token, url: BACKEND_URL });
    console.log('[TauriSocket] Rust socket connecting');
  } catch (error) {
    console.error('[TauriSocket] Failed to connect Rust socket:', error);
  }
}

/**
 * Disconnect the Rust-native Socket.io client.
 */
export async function disconnectRustSocket(): Promise<void> {
  if (!isTauri()) return;

  try {
    await invoke('runtime_socket_disconnect');
    console.log('[TauriSocket] Rust socket disconnected');
  } catch (error) {
    console.error('[TauriSocket] Failed to disconnect Rust socket:', error);
  }
}

/**
 * Emit an event through the Rust socket to the server.
 * Use this when the frontend needs to send events in Tauri mode.
 */
export async function emitViaRustSocket(event: string, data?: unknown): Promise<void> {
  if (!isTauri()) return;

  try {
    await invoke('runtime_socket_emit', { event, data: data ?? null });
  } catch (error) {
    console.error('[TauriSocket] Failed to emit via Rust socket:', error);
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

  try {
    return await invoke('runtime_socket_state');
  } catch (error) {
    console.error('[TauriSocket] Failed to get Rust socket state:', error);
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
  if (!isTauri()) return;

  try {
    // Listen for Rust socket state changes (Phase 2 — primary)
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
      }
    );

    // Listen for forwarded server events
    unlistenServerEvent = await listen<{ event: string; data: unknown }>('server:event', event => {
      console.log('[TauriSocket] Server event:', event.payload.event, event.payload.data);
      // Future: dispatch to specific handlers based on event type
    });

    // Legacy: Listen for connect requests from Rust (backwards compat)
    unlistenConnect = await listen<{ backendUrl: string; token: string }>(
      'socket:should_connect',
      async event => {
        console.log('[TauriSocket] Legacy connect request (ignored — using Rust socket)');
        // No-op: Rust socket handles connection now
        void event;
      }
    );

    // Legacy: Listen for disconnect requests from Rust
    unlistenDisconnect = await listen('socket:should_disconnect', async () => {
      console.log('[TauriSocket] Legacy disconnect request (ignored — using Rust socket)');
      // No-op: Rust socket handles disconnection now
    });

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
}

// ---------------------------------------------------------------------------
// Legacy reporting functions (kept for backwards compatibility)
// ---------------------------------------------------------------------------

/**
 * Report socket connected status to Rust (legacy — used in web mode).
 */
export async function reportSocketConnected(socketId?: string): Promise<void> {
  if (!isTauri()) return;

  try {
    await invoke('report_socket_connected', { socketId: socketId ?? null });
  } catch (error) {
    console.error('[TauriSocket] Failed to report connected:', error);
  }
}

/**
 * Report socket disconnected status to Rust (legacy — used in web mode).
 */
export async function reportSocketDisconnected(): Promise<void> {
  if (!isTauri()) return;

  try {
    await invoke('report_socket_disconnected');
  } catch (error) {
    console.error('[TauriSocket] Failed to report disconnected:', error);
  }
}

/**
 * Report socket error to Rust (legacy — used in web mode).
 */
export async function reportSocketError(error: string): Promise<void> {
  if (!isTauri()) return;

  try {
    await invoke('report_socket_error', { error });
  } catch (error) {
    console.error('[TauriSocket] Failed to report error:', error);
  }
}

/**
 * Update socket status in Rust (legacy — used in web mode).
 */
export async function updateSocketStatus(
  status: 'connected' | 'connecting' | 'disconnected' | 'reconnecting' | 'error',
  socketId?: string
): Promise<void> {
  if (!isTauri()) return;

  try {
    await invoke('update_socket_status', { status, socketId: socketId ?? null });
  } catch (error) {
    console.error('[TauriSocket] Failed to update status:', error);
  }
}

/**
 * Get session token from Rust secure storage.
 */
export async function getSecureToken(): Promise<string | null> {
  if (!isTauri()) return null;

  try {
    return await invoke<string | null>('get_session_token');
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
    return await invoke<boolean>('is_authenticated');
  } catch (error) {
    console.error('[TauriSocket] Failed to check auth:', error);
    return false;
  }
}
