/**
 * Skill runtime — higher-level wrapper around SkillTransport
 * for managing a single skill's lifecycle.
 */

import { invoke } from "@tauri-apps/api/core";
import { SkillTransport, type ReverseRpcHandler } from "./transport";
import type {
  SkillManifest,
  SetupStep,
  SetupResult,
  SkillToolDefinition,
  SkillOptionDefinition,
} from "./types";
import { getSkillModulePath } from "./paths";

/** Map runtime names to Tauri shell command names */
const SHELL_COMMANDS: Record<string, string> = {
  python: "runtime-skill-python",
  node: "runtime-skill-node",
  deno: "runtime-skill-deno",
};

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
   */
  onReverseRpc(handler: ReverseRpcHandler): void {
    this.transport.onReverseRpc(handler);
  }


  /**
   * Spawn the skill subprocess.
   * Uses absolute cwd from the backend so the sidecar finds the skills package.
   */
  async start(envVars?: Record<string, string>): Promise<void> {
    const shellName = SHELL_COMMANDS[this.manifest.runtime];
    if (!shellName) {
      throw new Error(`Unsupported runtime: ${this.manifest.runtime}`);
    }

    const modulePath = getSkillModulePath(this.manifest.id);
    const args = ["-m", modulePath];
    const cwd = await invoke<string>("skill_cwd");

    // Merge env vars
    const env: Record<string, string> = {};
    if (envVars) {
      Object.assign(env, envVars);
    }

    // In dev, add venv site-packages and cwd to PYTHONPATH for Python skills
    if (this.manifest.runtime === "python") {
      // Ensure cwd is absolute (it should be from skill_cwd, but double-check)
      const absCwd = cwd.startsWith("/") ? cwd : cwd;

      // Resolve venv site-packages path dynamically (Rust scans .venv/lib/)
      let venvSitePackages = "";
      try {
        venvSitePackages = await invoke<string>("skill_venv_site_packages");
      } catch {
        // Fallback if command fails
        venvSitePackages = `${absCwd}/.venv/lib/python3/site-packages`;
      }

      // Add cwd to PYTHONPATH so Python can find the 'skills' package (at cwd/skills/)
      // The skills package is at cwd/skills/, so cwd itself needs to be in PYTHONPATH
      const existingPythonPath = env.PYTHONPATH || "";
      const pythonPathParts = [absCwd, venvSitePackages];
      if (existingPythonPath) {
        pythonPathParts.push(existingPythonPath);
      }
      env.PYTHONPATH = pythonPathParts.join(":");

      console.log("[skill-runtime] Python env setup:", {
        cwd: absCwd,
        PYTHONPATH: env.PYTHONPATH,
        venvSitePackages,
        skillsPackageExists: `${absCwd}/skills`,
      });
    }

    await this.transport.start(shellName, args, env, cwd);
    this._started = true;
  }

  /**
   * Send skill/load with manifest + data dir + session data.
   */
  async load(additionalParams?: Record<string, unknown>): Promise<void> {
    // Use absolute path from Rust to avoid cwd-relative ambiguity
    const dataDir = await invoke<string>("skill_data_dir", {
      skillId: this.manifest.id,
    });
    await this.transport.request("skill/load", {
      manifest: this.manifest,
      dataDir,
      ...additionalParams,
    });
  }

  /**
   * Start the setup flow. Returns the first SetupStep.
   */
  async setupStart(): Promise<SetupStep> {
    const result = await this.transport.request<{ step: SetupStep }>(
      "setup/start"
    );
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
    return this.transport.request("tools/call", { name, arguments: args });
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
   * Unload and kill the skill process.
   */
  async stop(): Promise<void> {
    if (!this._started) return;
    try {
      await this.transport.request("skill/shutdown");
    } catch {
      // Process may already be dead
    }
    // Give process a moment to exit cleanly
    await new Promise((r) => setTimeout(r, 200));
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
