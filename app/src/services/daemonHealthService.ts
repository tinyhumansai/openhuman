/**
 * Daemon Health Service
 *
 * Polls the Rust core health snapshot and keeps the frontend daemon store in sync.
 */
import {
  type ComponentHealth,
  type HealthSnapshot,
  setDaemonStatus,
  updateHealthSnapshot,
} from '../features/daemon/store';
import { getCoreStateSnapshot } from '../lib/coreState/store';
import { callCoreRpc } from './coreRpcClient';

export class DaemonHealthService {
  private healthTimeoutId: ReturnType<typeof setTimeout> | null = null;
  private readonly HEALTH_TIMEOUT_MS = 30000;
  private pollingIntervalId: ReturnType<typeof setInterval> | null = null;
  private readonly POLL_MS = 2000;

  async setupHealthListener(): Promise<(() => void) | null> {
    if (this.pollingIntervalId) {
      return () => this.cleanup();
    }

    const pollOnce = async () => {
      try {
        const payload = await callCoreRpc<unknown>({ method: 'openhuman.health_snapshot' });
        const healthSnapshot = this.parseHealthSnapshot(payload);
        if (healthSnapshot) {
          this.updateDaemonStoreFromHealth(healthSnapshot);
          this.startHealthTimeout();
        }
      } catch {
        // The health endpoint can fail while the sidecar is starting.
      }
    };

    await pollOnce();
    this.pollingIntervalId = setInterval(() => {
      void pollOnce();
    }, this.POLL_MS);
    this.startHealthTimeout();

    return () => this.cleanup();
  }

  cleanup(): void {
    if (this.pollingIntervalId) {
      clearInterval(this.pollingIntervalId);
      this.pollingIntervalId = null;
    }

    if (this.healthTimeoutId) {
      clearTimeout(this.healthTimeoutId);
      this.healthTimeoutId = null;
    }
  }

  private parseHealthSnapshot(payload: unknown): HealthSnapshot | null {
    try {
      if (!payload || typeof payload !== 'object') {
        return null;
      }

      const data = payload as Record<string, unknown>;
      if (
        typeof data.pid !== 'number' ||
        typeof data.updated_at !== 'string' ||
        typeof data.uptime_seconds !== 'number' ||
        !data.components ||
        typeof data.components !== 'object'
      ) {
        return null;
      }

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

        if (comp.status !== 'ok' && comp.status !== 'error' && comp.status !== 'starting') {
          continue;
        }

        components[name] = {
          status: comp.status,
          updated_at: comp.updated_at,
          last_ok: typeof comp.last_ok === 'string' ? comp.last_ok : undefined,
          last_error: typeof comp.last_error === 'string' ? comp.last_error : undefined,
          restart_count: comp.restart_count,
        };
      }

      return {
        pid: data.pid,
        updated_at: data.updated_at,
        uptime_seconds: data.uptime_seconds,
        components,
      };
    } catch (error) {
      console.error('[DaemonHealth] Error parsing health snapshot:', error);
      return null;
    }
  }

  private updateDaemonStoreFromHealth(snapshot: HealthSnapshot): void {
    try {
      updateHealthSnapshot(this.getUserId(), snapshot);
    } catch (error) {
      console.error('[DaemonHealth] Error updating daemon store from health:', error);
    }
  }

  private startHealthTimeout(): void {
    if (this.healthTimeoutId) {
      clearTimeout(this.healthTimeoutId);
    }

    const userId = this.getUserId();
    this.healthTimeoutId = setTimeout(() => {
      console.warn('[DaemonHealth] Health timeout reached - setting status to disconnected');
      setDaemonStatus(userId, 'disconnected');
      this.healthTimeoutId = null;
    }, this.HEALTH_TIMEOUT_MS);
  }

  private getUserId(): string {
    const token = getCoreStateSnapshot().snapshot.sessionToken;
    if (!token) {
      return '__pending__';
    }

    try {
      const parts = token.split('.');
      if (parts.length !== 3) {
        return '__pending__';
      }

      const payloadBase64 = parts[1].replace(/-/g, '+').replace(/_/g, '/');
      const payloadJson = atob(payloadBase64);
      const payload = JSON.parse(payloadJson) as {
        sub?: string;
        tgUserId?: string;
        userId?: string;
      };
      return payload.tgUserId || payload.userId || payload.sub || '__pending__';
    } catch {
      return '__pending__';
    }
  }
}

export const daemonHealthService = new DaemonHealthService();
