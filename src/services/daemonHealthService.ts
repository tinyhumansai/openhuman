/**
 * Daemon Health Service
 *
 * Manages health monitoring for the openhuman daemon by listening to
 * 'openhuman:health' events emitted by the Rust backend every 5 seconds.
 */
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import { store } from '../store';
import {
  type ComponentHealth,
  type HealthSnapshot,
  setDaemonStatus,
  setHealthTimeoutId,
  updateHealthSnapshot,
} from '../store/daemonSlice';

export class DaemonHealthService {
  private healthTimeoutId: ReturnType<typeof setTimeout> | null = null;
  private readonly HEALTH_TIMEOUT_MS = 30000; // 30 seconds
  private healthEventListener: UnlistenFn | null = null;

  /**
   * Setup health event listener from the Rust daemon.
   * Should be called once when the app starts in Tauri mode.
   */
  async setupHealthListener(): Promise<UnlistenFn | null> {
    console.log('[DaemonHealth] setupHealthListener() called - starting setup process');
    try {
      // console.log('[DaemonHealth] About to call listen() for openhuman:health event');
      // console.log('[DaemonHealth] Setting up openhuman:health event listener');

      this.healthEventListener = await listen<unknown>('openhuman:health', event => {
        // console.log('[DaemonHealth] Received health event:', event.payload);

        const healthSnapshot = this.parseHealthSnapshot(event.payload);
        if (healthSnapshot) {
          this.updateReduxFromHealth(healthSnapshot);
          this.startHealthTimeout();
        } else {
          console.warn('[DaemonHealth] Failed to parse health snapshot:', event.payload);
        }
      });
      console.log('[DaemonHealth] openhuman:health listener created successfully');

      // Start initial timeout
      // console.log('[DaemonHealth] Starting health timeout');
      this.startHealthTimeout();
      // console.log('[DaemonHealth] Health timeout started');

      // console.log('[DaemonHealth] Health listener setup complete');
      return this.healthEventListener;
    } catch (error) {
      console.error('[DaemonHealth] Failed to setup health listener:', error);
      return null;
    }
  }

  /**
   * Cleanup the health event listener.
   */
  cleanup(): void {
    if (this.healthEventListener) {
      this.healthEventListener();
      this.healthEventListener = null;
    }

    if (this.healthTimeoutId) {
      clearTimeout(this.healthTimeoutId);
      this.healthTimeoutId = null;
    }
  }

  /**
   * Parse the health snapshot received from Rust.
   */
  private parseHealthSnapshot(payload: unknown): HealthSnapshot | null {
    try {
      if (!payload || typeof payload !== 'object') {
        return null;
      }

      const data = payload as Record<string, unknown>;

      // Validate required fields
      if (
        typeof data.pid !== 'number' ||
        typeof data.updated_at !== 'string' ||
        typeof data.uptime_seconds !== 'number' ||
        !data.components ||
        typeof data.components !== 'object'
      ) {
        return null;
      }

      // Parse components
      const components: Record<string, ComponentHealth> = {};
      const componentsData = data.components as Record<string, unknown>;

      for (const [name, component] of Object.entries(componentsData)) {
        if (!component || typeof component !== 'object') {
          continue;
        }

        const comp = component as Record<string, unknown>;
        if (
          typeof comp.status !== 'string' ||
          typeof comp.updated_at !== 'string' ||
          typeof comp.restart_count !== 'number'
        ) {
          continue;
        }

        // Validate status is a valid ComponentStatus
        if (comp.status !== 'ok' && comp.status !== 'error' && comp.status !== 'starting') {
          continue;
        }

        components[name] = {
          status: comp.status as 'ok' | 'error' | 'starting',
          updated_at: comp.updated_at,
          last_ok: typeof comp.last_ok === 'string' ? comp.last_ok : undefined,
          last_error: typeof comp.last_error === 'string' ? comp.last_error : undefined,
          restart_count: comp.restart_count,
        };
      }

      return {
        pid: data.pid as number,
        updated_at: data.updated_at as string,
        uptime_seconds: data.uptime_seconds as number,
        components,
      };
    } catch (error) {
      console.error('[DaemonHealth] Error parsing health snapshot:', error);
      return null;
    }
  }

  /**
   * Update Redux state based on received health snapshot.
   */
  private updateReduxFromHealth(snapshot: HealthSnapshot): void {
    const userId = this.getUserId();

    try {
      // Update the health snapshot in Redux
      store.dispatch(updateHealthSnapshot({ userId, healthSnapshot: snapshot }));

      // console.log('[DaemonHealth] Updated health snapshot for user:', userId, snapshot);
    } catch (error) {
      console.error('[DaemonHealth] Error updating Redux from health:', error);
    }
  }

  /**
   * Start or restart the health timeout.
   * If no health events are received within the timeout period,
   * the daemon status will be set to 'disconnected'.
   */
  private startHealthTimeout(): void {
    // Clear existing timeout
    if (this.healthTimeoutId) {
      clearTimeout(this.healthTimeoutId);
    }

    const userId = this.getUserId();

    // Set new timeout
    this.healthTimeoutId = setTimeout(() => {
      console.warn('[DaemonHealth] Health timeout reached - setting status to disconnected');
      store.dispatch(setDaemonStatus({ userId, status: 'disconnected' }));
      store.dispatch(setHealthTimeoutId({ userId, timeoutId: null }));
      this.healthTimeoutId = null;
    }, this.HEALTH_TIMEOUT_MS);

    // Store timeout ID in Redux for cleanup
    store.dispatch(setHealthTimeoutId({ userId, timeoutId: this.healthTimeoutId.toString() }));
  }

  /**
   * Get the current user ID for daemon state management.
   */
  private getUserId(): string {
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
}

// Export singleton instance
export const daemonHealthService = new DaemonHealthService();
