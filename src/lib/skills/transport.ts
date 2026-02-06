/**
 * JSON-RPC 2.0 transport over Tauri IPC commands.
 *
 * Routes JSON-RPC requests to the Rust QuickJS runtime engine via
 * `invoke('runtime_rpc', ...)`. Reverse RPC (state/get, state/set,
 * data/read, data/write) is handled by bridge globals inside the
 * QuickJS engine.
 */

import { invoke } from '@tauri-apps/api/core';

export type ReverseRpcHandler = (
  method: string,
  params: Record<string, unknown>
) => Promise<unknown>;

export class SkillTransport {
  private skillId: string | null = null;
  private _started = false;

  /**
   * Set a handler for reverse RPC calls from the skill process.
   * With QuickJS, reverse RPC is handled by bridge globals, so this
   * is kept for API compatibility but is a no-op.
   */
  onReverseRpc(_handler: ReverseRpcHandler): void {
    // No-op: QuickJS bridge globals handle state/data directly
  }

  /**
   * Initialize the transport for a skill.
   * With QuickJS, the skill process is managed by the Rust runtime engine,
   * so this just stores the skill ID for routing RPC calls.
   *
   * @param skillId - The skill ID to route requests to.
   */
  async start(skillId: string): Promise<void> {
    this.skillId = skillId;
    this._started = true;
    console.log("[skill-transport] Initialized for skill:", skillId);
  }

  /**
   * Send a JSON-RPC request to the skill via Tauri IPC.
   */
  async request<T = unknown>(
    method: string,
    params?: Record<string, unknown>
  ): Promise<T> {
    if (!this.skillId || !this._started) {
      throw new Error("Skill transport not started");
    }

    console.log("[skill-transport] Sending request", {
      skillId: this.skillId,
      method,
      hasParams: params !== undefined,
    });

    const result = await invoke<T>("runtime_rpc", {
      skillId: this.skillId,
      method,
      params: params ?? {},
    });

    console.debug("[skill-transport] Received response", {
      skillId: this.skillId,
      method,
    });

    return result;
  }

  /**
   * Send a JSON-RPC notification (no response expected).
   * Implemented as a fire-and-forget request.
   */
  notify(method: string, params?: Record<string, unknown>): void {
    if (!this.skillId || !this._started) {
      console.warn("[skill-transport] Cannot notify - not started", { method });
      return;
    }

    console.log("[skill-transport] Sending notification", {
      skillId: this.skillId,
      method,
    });

    // Fire and forget
    invoke("runtime_rpc", {
      skillId: this.skillId,
      method,
      params: params ?? {},
    }).catch((err: unknown) => {
      console.error("[skill-transport] Notification error:", err);
    });
  }

  /**
   * Stop the transport. With QuickJS, this is a no-op since the
   * Rust engine manages skill lifecycle. Use runtime_stop_skill instead.
   */
  async kill(): Promise<void> {
    if (this.skillId && this._started) {
      try {
        await invoke("runtime_stop_skill", { skillId: this.skillId });
      } catch {
        // Skill may already be stopped
      }
    }
    this._started = false;
  }

  get isRunning(): boolean {
    return this._started;
  }
}
