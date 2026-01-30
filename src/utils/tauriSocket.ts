/**
 * Tauri Socket Integration
 *
 * This module provides integration between the Rust backend and the frontend
 * Socket.io service. In Tauri, the socket stays connected even when the
 * window is hidden because the WebView is not destroyed.
 *
 * The Rust backend emits events to coordinate:
 * - socket:should_connect - When Rust wants frontend to connect
 * - socket:should_disconnect - When Rust wants frontend to disconnect
 *
 * The frontend reports status back:
 * - report_socket_connected - When connection established
 * - report_socket_disconnected - When disconnected
 * - report_socket_error - When error occurs
 */

import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { invoke, isTauri as coreIsTauri } from '@tauri-apps/api/core';
import { socketService } from '../services/socketService';

// Check if we're running in Tauri
export const isTauri = (): boolean => {
  return coreIsTauri();
};

let unlistenConnect: UnlistenFn | null = null;
let unlistenDisconnect: UnlistenFn | null = null;

/**
 * Setup Tauri socket event listeners
 * This should be called once when the app starts
 */
export async function setupTauriSocketListeners(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  try {
    // Listen for connect requests from Rust
    unlistenConnect = await listen<{ backendUrl: string; token: string }>(
      'socket:should_connect',
      async (event) => {
        console.log('[TauriSocket] Received connect request');
        const { token } = event.payload;

        try {
          socketService.connect(token);
          // Note: We'll report connected status when socket actually connects
        } catch (error) {
          console.error('[TauriSocket] Failed to connect:', error);
          await invoke('report_socket_error', {
            error: error instanceof Error ? error.message : 'Connection failed'
          });
        }
      }
    );

    // Listen for disconnect requests from Rust
    unlistenDisconnect = await listen(
      'socket:should_disconnect',
      async () => {
        console.log('[TauriSocket] Received disconnect request');
        socketService.disconnect();
        await invoke('report_socket_disconnected');
      }
    );

    console.log('[TauriSocket] Event listeners setup complete');
  } catch (error) {
    console.error('[TauriSocket] Failed to setup listeners:', error);
  }
}

/**
 * Cleanup Tauri socket event listeners
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
}

/**
 * Report socket connected status to Rust
 */
export async function reportSocketConnected(socketId?: string): Promise<void> {
  if (!isTauri()) {
    return;
  }

  try {
    await invoke('report_socket_connected', { socketId: socketId ?? null });
    console.log('[TauriSocket] Reported connected');
  } catch (error) {
    console.error('[TauriSocket] Failed to report connected:', error);
  }
}

/**
 * Report socket disconnected status to Rust
 */
export async function reportSocketDisconnected(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  try {
    await invoke('report_socket_disconnected');
    console.log('[TauriSocket] Reported disconnected');
  } catch (error) {
    console.error('[TauriSocket] Failed to report disconnected:', error);
  }
}

/**
 * Report socket error to Rust
 */
export async function reportSocketError(error: string): Promise<void> {
  if (!isTauri()) {
    return;
  }

  try {
    await invoke('report_socket_error', { error });
    console.log('[TauriSocket] Reported error:', error);
  } catch (error) {
    console.error('[TauriSocket] Failed to report error:', error);
  }
}

/**
 * Update socket status in Rust
 */
export async function updateSocketStatus(
  status: 'connected' | 'connecting' | 'disconnected' | 'reconnecting' | 'error',
  socketId?: string
): Promise<void> {
  if (!isTauri()) {
    return;
  }

  try {
    await invoke('update_socket_status', { status, socketId: socketId ?? null });
  } catch (error) {
    console.error('[TauriSocket] Failed to update status:', error);
  }
}

/**
 * Get session token from Rust secure storage
 */
export async function getSecureToken(): Promise<string | null> {
  if (!isTauri()) {
    return null;
  }

  try {
    return await invoke<string | null>('get_session_token');
  } catch (error) {
    console.error('[TauriSocket] Failed to get secure token:', error);
    return null;
  }
}

/**
 * Check if user is authenticated (from Rust)
 */
export async function isAuthenticatedFromRust(): Promise<boolean> {
  if (!isTauri()) {
    return false;
  }

  try {
    return await invoke<boolean>('is_authenticated');
  } catch (error) {
    console.error('[TauriSocket] Failed to check auth:', error);
    return false;
  }
}
