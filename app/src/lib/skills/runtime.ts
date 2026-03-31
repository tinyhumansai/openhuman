/**
 * Skill runtime — higher-level wrapper around SkillTransport
 * for managing a single skill's lifecycle.
 *
 * Skills are managed by the Rust QuickJS runtime engine.
 * This class wraps the transport layer to provide the same API
 * that the SkillManager expects.
 */

import { callCoreRpc } from "../../services/coreRpcClient";
import { runtimeSkillDataDir } from "../../utils/tauriCommands";
import { SkillTransport, type ReverseRpcHandler } from "./transport";
import type {
  SkillManifest,
  SetupStep,
  SetupResult,
  SkillToolDefinition,
  SkillOptionDefinition,
  PingResult,
} from "./types";

export class SkillRuntime {
  private transport: SkillTransport;
  private manifest: SkillManifest;
  private _started = false;

  constructor(manifest: SkillManifest) {
    this.transport = new SkillTransport();
    this.manifest = manifest;
  }

  /**
   * Set a handler for reverse RPC calls from the skill process.
   * Reverse RPC is handled by bridge globals, so this
   * is kept for API compatibility.
   */
  onReverseRpc(handler: ReverseRpcHandler): void {
    this.transport.onReverseRpc(handler);
  }


  /**
   * Start the skill in the QuickJS runtime engine.
   * The Rust engine handles process management, so we just tell it to start
   * and then initialize the transport for RPC routing.
   */
  async start(): Promise<void> {
    // Start the skill in the Rust QuickJS runtime via core RPC
    await callCoreRpc({
      method: 'openhuman.skills_start',
      params: { skill_id: this.manifest.id },
    });

    // Initialize the transport for RPC routing
    await this.transport.start(this.manifest.id);
    this._started = true;
  }

  /**
   * Send skill/load with manifest + data dir.
   * Loading is handled by the Rust engine during start_skill,
   * so this sends a no-op skill/load RPC for protocol compatibility.
   */
  async load(additionalParams?: Record<string, unknown>): Promise<void> {
    const dataDir = await runtimeSkillDataDir(this.manifest.id);
    await this.transport.request("skill/load", {
      manifest: this.manifest,
      dataDir,
      ...(additionalParams || {}),
    });
  }

  /**
   * Start the setup flow. Returns the first SetupStep.
   * Returns null if the skill does not implement setup/start (e.g. OAuth-only skills).
   */
  async setupStart(): Promise<SetupStep | null> {
    console.log("[SkillRuntime] setupStart", this.skillId);
    const result = await this.transport.request<{ step: SetupStep } | null>(
      "setup/start"
    );
    console.log("[SkillRuntime] setupStart result", this.skillId, result);
    if (!result || !result.step) {
      return null;
    }
    return result.step;
  }

  /**
   * Submit a setup step. Returns SetupResult with next/error/complete.
   */
  async setupSubmit(
    stepId: string,
    values: Record<string, unknown>
  ): Promise<SetupResult> {
    return this.transport.request<SetupResult>("setup/submit", {
      stepId,
      values,
    });
  }

  /**
   * Cancel the setup flow.
   */
  async setupCancel(): Promise<void> {
    await this.transport.request("setup/cancel");
  }

  /**
   * List available tools.
   */
  async listTools(): Promise<SkillToolDefinition[]> {
    const result = await this.transport.request<{
      tools: SkillToolDefinition[];
    }>("tools/list");
    return result.tools;
  }

  /**
   * List runtime-configurable options with current values.
   */
  async listOptions(): Promise<SkillOptionDefinition[]> {
    const result = await this.transport.request<{
      options: SkillOptionDefinition[];
    }>("options/list");
    return result.options;
  }

  /**
   * Set a single option value.
   */
  async setOption(name: string, value: unknown): Promise<void> {
    await this.transport.request("options/set", { name, value });
  }

  /**
   * Call a tool by name with arguments.
   */
  async callTool(
    name: string,
    args: Record<string, unknown>
  ): Promise<{
    content: Array<{ type: string; text: string }>;
    isError: boolean;
  }> {
    console.log(`[SkillRuntime] callTool skill="${this.manifest.id}" tool="${name}"`);
    const result = await this.transport.request<{ content: Array<{ type: string; text: string }>; isError: boolean }>("tools/call", { name, arguments: args });
    console.log(`[SkillRuntime] tools/call response skill="${this.manifest.id}" tool="${name}" isError=${result.isError}`);
    return result;
  }

  /**
   * Ping the skill to verify its external service connection is healthy.
   * Returns null if the skill doesn't implement onPing (backward compatible).
   */
  async ping(): Promise<PingResult | null> {
    return this.transport.request<PingResult | null>("skill/ping");
  }

  /**
   * Trigger the skill's onSync lifecycle hook.
   * Progress updates flow via published state fields, not the RPC response.
   */
  async triggerSync(): Promise<unknown> {
    return this.transport.request("skill/sync");
  }

  /**
   * Trigger periodic tick.
   */
  async tick(): Promise<void> {
    await this.transport.request("skill/tick");
  }

  /**
   * Notify skill of session start.
   */
  async sessionStart(sessionId: string): Promise<void> {
    await this.transport.request("skill/sessionStart", { sessionId });
  }

  /**
   * Notify skill of session end.
   */
  async sessionEnd(sessionId: string): Promise<void> {
    await this.transport.request("skill/sessionEnd", { sessionId });
  }

  /**
   * Notify the skill that OAuth completed successfully.
   * Sets the credential on the bridge and calls onOAuthComplete.
   */
  async oauthComplete(args: {
    credentialId: string;
    provider: string;
    grantedScopes?: string[];
    accountLabel?: string;
  }): Promise<void> {
    await this.transport.request("oauth/complete", args as unknown as Record<string, unknown>);
  }

  /**
   * Notify the skill that an OAuth credential was revoked.
   */
  async oauthRevoked(args: {
    credentialId: string;
    reason: string;
  }): Promise<void> {
    await this.transport.request("oauth/revoked", args as unknown as Record<string, unknown>);
  }

  /**
   * Unload and stop the skill.
   */
  async stop(): Promise<void> {
    if (!this._started) return;
    try {
      await this.transport.request("skill/shutdown");
    } catch {
      // Skill may already be stopped
    }
    await this.transport.kill();
    this._started = false;
  }

  get isRunning(): boolean {
    return this._started && this.transport.isRunning;
  }

  get skillId(): string {
    return this.manifest.id;
  }
}
