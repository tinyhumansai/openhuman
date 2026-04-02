/**
 * Skill Manager — orchestrates multiple skill runtimes.
 *
 * Singleton that manages skill discovery, lifecycle, setup flows,
 * and tool invocation. Dispatches status changes to Redux.
 */

import { callCoreRpc } from "../../services/coreRpcClient";
import { SkillRuntime } from "./runtime";
import { emitSkillStateChange } from "./skillEvents";
import {
  getSkillSnapshot,
  setSetupComplete as rpcSetSetupComplete,
  revokeOAuth as rpcRevokeOAuth,
  removePersistedOAuthCredential,
} from "./skillsApi";
import { syncToolsToBackend } from "./sync";
import type {
  SkillManifest,
  SkillStatus,
  SetupStep,
  SetupResult,
  SkillToolDefinition,
  SkillOptionDefinition,
} from "./types";
import { store } from "../../store";
import { setPrimaryWalletAddressForUser } from "../../store/authSlice";
import {
  runtimeSkillDataDir,
  runtimeSkillDataRead,
  runtimeSkillDataWrite,
} from "../../utils/tauriCommands";
import { toolExecutionTimeoutMsFromEnv, withTimeout } from "../../utils/withTimeout";
// Env vars kept for reverse RPC compatibility (may be used by skills via state)


class SkillManager {
  private runtimes = new Map<string, SkillRuntime>();

  /**
   * Get skill-specific load parameters (e.g., wallet address for wallet skill)
   */
  private getSkillLoadParams(skillId: string): Record<string, unknown> {
    const params: Record<string, unknown> = {};

    if (skillId === "wallet") {
      const state = store.getState();
      const userId = state.user.user?._id;
      const primaryAddress =
        userId && state.auth.primaryWalletAddressByUser?.[userId];
      if (primaryAddress) {
        params.walletAddress = primaryAddress;
      }
    }

    return params;
  }

  /**
   * Add a discovered skill manifest to Redux.
   */
  registerSkill(manifest: SkillManifest): void {
    if (manifest.id.includes("_")) {
      console.error(
        `Skill name "${manifest.id}" contains underscore. Skill names cannot contain underscores as they are used for tool namespacing (skillId__toolName).`
      );
      return;
    }
    // Registration is now handled by the Rust engine; just notify hooks
    emitSkillStateChange(manifest.id);
  }

  /**
   * Start a skill — spawn process, load, check setup status.
   * If setup is already complete, loads the skill fully and lists tools.
   */
  async startSkill(manifest: SkillManifest): Promise<void> {
    const skillId = manifest.id;

    if (this.runtimes.has(skillId)) {
      const existing = this.runtimes.get(skillId)!;
      if (existing.isRunning) return;
      this.runtimes.delete(skillId);
    }

    emitSkillStateChange(skillId);

    const runtime = new SkillRuntime(manifest);
    runtime.onReverseRpc(async (method, params) => {
      return this.handleReverseRpc(skillId, method, params);
    });

    try {
      await runtime.start();
      this.runtimes.set(skillId, runtime);

      const loadParams = this.getSkillLoadParams(manifest.id);
      await runtime.load(loadParams);

      // If no setup required, mark complete and activate
      if (!manifest.setup?.required) {
        await rpcSetSetupComplete(skillId, true).catch(() => {});
        await this.activateSkill(skillId);
      }

      emitSkillStateChange(skillId);
    } catch (err) {
      this.runtimes.delete(skillId);
      emitSkillStateChange(skillId);
      throw err;
    }
  }

  /**
   * After realtime socket reconnect: refresh tool lists for every running skill so
   * `tool:sync` matches the Rust engine (issue #215).
   */
  async resyncRunningSkillsAfterReconnect(): Promise<void> {
    const ids = [...this.runtimes.keys()];
    await Promise.all(ids.map((id) => this.activateSkill(id)));
  }

  /**
   * Activate a skill that has completed setup — list its tools and mark as ready.
   */
  private async activateSkill(skillId: string): Promise<void> {
    const runtime = this.runtimes.get(skillId);
    if (!runtime) return;

    try {
      await runtime.listTools();
      // Tools are tracked by the Rust engine; just sync to backend and notify hooks
      syncToolsToBackend();
      emitSkillStateChange(skillId);
    } catch (err) {
      console.error(`[SkillManager] Failed to activate skill ${skillId}:`, err);
      emitSkillStateChange(skillId);
    }
  }

  /**
   * Start the setup flow for a skill. Returns the first step, or null if
   * the skill doesn't implement setup/start (e.g. OAuth-only skills).
   */
  async startSetup(skillId: string): Promise<SetupStep | null> {
    console.log("[SkillManager] startSetup", skillId);
    const runtime = this.runtimes.get(skillId);
    if (!runtime) {
      console.log("[SkillManager] runtime not found", skillId);
      throw new Error(`Skill ${skillId} runtime not found`);
    }

    emitSkillStateChange(skillId);
    console.log("[SkillManager] setup started", skillId);
    return runtime.setupStart();
  }

  /**
   * Submit a setup step. Returns the result (next step, error, or complete).
   */
  async submitSetup(
    skillId: string,
    stepId: string,
    values: Record<string, unknown>,
  ): Promise<SetupResult> {
    const runtime = this.runtimes.get(skillId);
    if (!runtime) {
      throw new Error(`Skill ${skillId} is not running`);
    }

    const result = await runtime.setupSubmit(stepId, values);

    if (result.status === "complete") {
      await rpcSetSetupComplete(skillId, true).catch(() => {});
      await this.activateSkill(skillId);
    }

    return result;
  }

  /**
   * Cancel the setup flow for a skill.
   */
  async cancelSetup(skillId: string): Promise<void> {
    const runtime = this.runtimes.get(skillId);
    if (!runtime) return;

    try {
      await runtime.setupCancel();
    } catch {
      // Ignore errors on cancel
    }
    emitSkillStateChange(skillId);
  }

  /**
   * Call a tool on a running skill.
   */
  async callTool(
    skillId: string,
    name: string,
    args: Record<string, unknown>,
  ): Promise<{ content: Array<{ type: string; text: string }>; isError: boolean }> {
    console.log(`[SkillManager] callTool skill="${skillId}" tool="${name}"`);
    const runtime = this.runtimes.get(skillId);
    if (!runtime) {
      console.error(`[SkillManager] callTool failed — skill "${skillId}" has no running runtime`);
      throw new Error(`Skill ${skillId} is not running`);
    }
    const timeoutMs = toolExecutionTimeoutMsFromEnv();
    const result = await withTimeout(
      runtime.callTool(name, args),
      timeoutMs,
      `[SkillManager] callTool skill="${skillId}" tool="${name}"`,
    );
    console.log(`[SkillManager] callTool result skill="${skillId}" tool="${name}" isError=${result.isError}`);
    return result;
  }

  /**
   * Get the list of tools for a running skill.
   */
  async listTools(skillId: string): Promise<SkillToolDefinition[]> {
    const runtime = this.runtimes.get(skillId);
    if (!runtime) {
      throw new Error(`Skill ${skillId} is not running`);
    }
    return runtime.listTools();
  }

  /**
   * List runtime-configurable options for a running skill.
   */
  async listOptions(skillId: string): Promise<SkillOptionDefinition[]> {
    const runtime = this.runtimes.get(skillId);
    if (!runtime) {
      throw new Error(`Skill ${skillId} is not running`);
    }
    return runtime.listOptions();
  }

  /**
   * Trigger a manual sync for a running skill.
   * Progress updates are published to Redux via the skill's state fields.
   */
  async triggerSync(skillId: string): Promise<void> {
    const timeoutMs = toolExecutionTimeoutMsFromEnv();
    const runtime = this.runtimes.get(skillId);
    if (runtime) {
      await withTimeout(
        runtime.triggerSync(),
        timeoutMs,
        `[SkillManager] triggerSync skill="${skillId}"`,
      );
    } else {
      // Try via core RPC pass-through
      try {
        await withTimeout(
          callCoreRpc({
            method: "openhuman.skills_sync",
            params: { skill_id: skillId },
          }),
          timeoutMs,
          `[SkillManager] skills_sync skill="${skillId}"`,
        );
      } catch {
        // Skill not running — skip sync silently
      }
    }
  }

  /**
   * Set a single option on a running skill.
   */
  async setOption(skillId: string, name: string, value: unknown): Promise<void> {
    const runtime = this.runtimes.get(skillId);
    if (!runtime) {
      throw new Error(`Skill ${skillId} is not running`);
    }
    await runtime.setOption(name, value);
    // Refresh tools list since tool_filter options can change available tools
    await this.activateSkill(skillId);
  }

  /**
   * Notify a skill that OAuth completed successfully.
   * Called by the deep link handler after backend OAuth callback.
   * For Gmail, pass extraCredential.accessToken so the skill uses the token directly.
   */
  async notifyOAuthComplete(
    skillId: string,
    integrationId: string,
    provider?: string,
    extraCredential?: { accessToken?: string },
  ): Promise<void> {
    // Persist setup completion via RPC (always, regardless of runtime)
    await rpcSetSetupComplete(skillId, true).catch(() => {});

    // Try to notify the local runtime if one exists
    const runtime = this.runtimes.get(skillId);
    if (runtime?.isRunning) {
      const credential = {
        credentialId: integrationId,
        provider: provider ?? "unknown",
        grantedScopes: [] as string[],
        ...extraCredential,
      };
      try {
        await runtime.oauthComplete(credential);
      } catch (err) {
        console.warn(`[SkillManager] oauthComplete RPC failed for ${skillId}:`, err);
      }
      await this.activateSkill(skillId);
    } else {
      // No local runtime — try notifying via core RPC pass-through.
      // The credential object must use `credentialId` (not `integrationId`)
      // to match what the JS bootstrap's oauth.fetch expects.
      try {
        await callCoreRpc({
          method: "openhuman.skills_rpc",
          params: {
            skill_id: skillId,
            method: "oauth/complete",
            params: {
              credentialId: integrationId,
              provider: provider ?? "unknown",
              grantedScopes: [] as string[],
              ...extraCredential,
            },
          },
        });
      } catch {
        // Skill may not be running in the core either — that's OK,
        // setup_complete is already persisted above
      }
    }

    emitSkillStateChange(skillId);
  }

  /**
   * Forward session start to all ready skills.
   */
  async sessionStart(sessionId: string): Promise<void> {
    for (const [, runtime] of this.runtimes) {
      if (runtime.isRunning) {
        try {
          await runtime.sessionStart(sessionId);
        } catch {
          // Non-critical
        }
      }
    }
  }

  /**
   * Forward session end to all ready skills.
   */
  async sessionEnd(sessionId: string): Promise<void> {
    for (const [, runtime] of this.runtimes) {
      if (runtime.isRunning) {
        try {
          await runtime.sessionEnd(sessionId);
        } catch {
          // Non-critical
        }
      }
    }
  }

  /**
   * Disconnect a skill — revoke OAuth credentials, stop it, and reset setup state.
   */
  async disconnectSkill(skillId: string): Promise<void> {
    // Read the stored credential ID so oauth/revoked clears the right memory bucket.
    let credentialId: string | undefined;
    try {
      const snap = await getSkillSnapshot(skillId);
      const cred = snap?.state?.__oauth_credential as
        | { credentialId?: string }
        | string
        | undefined;
      if (cred && typeof cred === "object") {
        credentialId = cred.credentialId;
      }
    } catch {
      // Snapshot may fail if skill isn't registered yet
    }

    // Revoke OAuth credential before stopping so the running skill can clean up
    // its in-memory state and the event loop deletes oauth_credential.json.
    let revokeSucceeded = false;
    try {
      await rpcRevokeOAuth(skillId, credentialId ?? "default");
      revokeSucceeded = true;
    } catch (err) {
      console.debug(
        "[SkillManager] oauth/revoked failed (runtime may be stopped):",
        err,
      );
    }

    try {
      await this.stopSkill(skillId);
    } finally {
      if (!revokeSucceeded) {
        await removePersistedOAuthCredential(skillId).catch((err) => {
          console.debug(
            "[SkillManager] host-side credential cleanup failed:",
            err,
          );
        });
      }
    }

    await rpcSetSetupComplete(skillId, false).catch(() => {});
    emitSkillStateChange(skillId);
    syncToolsToBackend();
  }

  /**
   * Stop a specific skill.
   */
  async stopSkill(skillId: string): Promise<void> {
    const runtime = this.runtimes.get(skillId);
    if (!runtime) return;

    try {
      await runtime.stop();
    } catch {
      // Ignore stop errors
    }
    this.runtimes.delete(skillId);
    emitSkillStateChange(skillId);
    syncToolsToBackend();
  }

  /**
   * Stop all running skills.
   */
  async stopAll(): Promise<void> {
    const ids = Array.from(this.runtimes.keys());
    await Promise.all(ids.map((id) => this.stopSkill(id)));
  }

  /**
   * Check if a skill is currently running.
   */
  isSkillRunning(skillId: string): boolean {
    return this.runtimes.get(skillId)?.isRunning ?? false;
  }

  /**
   * Get the current status of a skill from Redux.
   */
  getSkillStatus(_skillId: string): SkillStatus | undefined {
    // Status is now tracked by the Rust engine; use useSkillSnapshot() hook for reads
    return undefined;
  }

  /**
   * Reload a skill with updated parameters (e.g., after authentication).
   */
  async reloadSkill(skillId: string): Promise<void> {
    const runtime = this.runtimes.get(skillId);
    if (!runtime || !runtime.isRunning) {
      return; // Skill not running, nothing to reload
    }

    try {
      // Get updated load parameters
      const loadParams = this.getSkillLoadParams(skillId);

      // Reload the skill with new parameters
      await runtime.load(loadParams);

      await this.activateSkill(skillId);
    } catch (err) {
      console.error(`Error reloading skill ${skillId}:`, err);
    }
  }

  /**
   * Set the wallet address in the frontend app and notify the wallet skill (onLoad).
   * Updates Redux (primaryWalletAddressByUser) and, if the wallet skill is running,
   * sends load params so the skill receives onLoad({ walletAddress }).
   */
  async setWalletAddress(address: string): Promise<void> {
    const state = store.getState();
    const userId = state.user.user?._id;
    if (!userId) {
      return;
    }
    store.dispatch(setPrimaryWalletAddressForUser({ userId, address }));
    const runtime = this.runtimes.get("wallet");
    if (runtime?.isRunning) {
      await runtime.load({ walletAddress: address });
    }
  }

  // -----------------------------------------------------------------------
  // Reverse RPC handling
  // -----------------------------------------------------------------------

  private async handleReverseRpc(
    skillId: string,
    method: string,
    params: Record<string, unknown>,
  ): Promise<unknown> {
    switch (method) {
      case "state/get":
        // State is managed by the Rust engine's published_state
        return { state: {} };

      case "state/set": {
        // State is managed by the Rust engine; just notify hooks to re-fetch
        emitSkillStateChange(skillId);
        syncToolsToBackend();
        return { ok: true };
      }

      case "data/read": {
        const filename = params.filename as string;
        try {
          const content = await runtimeSkillDataRead(skillId, filename);
          return { content };
        } catch {
          return { content: "" };
        }
      }

      case "data/write": {
        const filename = params.filename as string;
        const content = params.content as string;
        try {
          await runtimeSkillDataWrite(skillId, filename, content);
        } catch (err) {
          console.error("[skill-manager] data/write error:", err);
        }
        return { ok: true };
      }

      case "intelligence/emitEvent":
        // Future: forward to intelligence system
        console.debug("[skill-manager] Intelligence event:", params);
        return { ok: true };

      case "entities/upsert":
        // Future: forward to entity manager
        console.debug("[skill-manager] Entity upsert:", params);
        return { ok: true };

      case "entities/search":
        // Future: forward to entity manager
        return { results: [] };

      case "entities/upsertRelationship":
        console.debug("[skill-manager] Relationship upsert:", params);
        return { ok: true };

      case "entities/getRelationships":
        return { results: [] };

      default:
        throw new Error(`Unknown reverse RPC method: ${method}`);
    }
  }

  /**
   * Clear all skills databases and cached data.
   * Used for nuclear reset functionality.
   */
  async clearAllSkillsData(): Promise<void> {
    try {
      // Stop all running skills first
      await this.stopAll();

      // Get all skill IDs from runtime map
      const skillIds = Array.from(this.runtimes.keys());

      // Clear data for each skill
      const clearPromises = skillIds.map(async (skillId) => {
        try {
          // Get skill data directory path
          const dataDir = await runtimeSkillDataDir(skillId);

          // Note: We don't directly delete directories here since there's no exposed
          // Tauri command for that. Instead, we rely on the backend to handle
          // clearing when skills are disabled/reset via Redux state clearing.

          console.log(`[SkillManager] Skill ${skillId} data directory: ${dataDir}`);
        } catch (err) {
          console.warn(`[SkillManager] Failed to get data directory for skill ${skillId}:`, err);
        }
      });

      await Promise.all(clearPromises);

      console.log("[SkillManager] Skills data clearing initiated");
    } catch (error) {
      console.error("[SkillManager] Failed to clear skills data:", error);
      throw new Error("Failed to clear skills databases");
    }
  }
}

// Export singleton
export const skillManager = new SkillManager();

// Debug: expose to window for console testing
if (typeof window !== 'undefined') {
  (window as unknown as { __skillManager: SkillManager }).__skillManager = skillManager;
}
