/**
 * Unified Tool Registry
 *
 * Replaces three fragmented tool systems:
 * - AI skills tools (src/lib/ai/tools/registry.ts)
 * - MCP extra tools (src/lib/mcp/skills/index.ts)
 * - Telegram MCP tools (src/lib/telegram/server.ts findToolHandler)
 *
 * All tools from all skills are registered here with their handlers.
 * Tool execution flow:
 *   1. Look up tool in registry
 *   2. Enforce rate limits (delegates to existing rateLimiter.ts)
 *   3. Build SkillToolContext for the owning skill
 *   4. Call handler
 *   5. Return result
 */

import type { MCPTool, MCPToolResult } from "../mcp/types";
import type { ToolTier } from "../mcp/rateLimiter";
import { enforceRateLimit, classifyTool } from "../mcp/rateLimiter";
import type {
  SkillToolDefinition,
  SkillToolContext,
  UnifiedToolRegistry,
} from "./types";
import type { SkillStateManager } from "./state-manager";
import createDebug from "debug";

const log = createDebug("app:skills:tools");

interface RegisteredTool {
  skillId: string;
  def: SkillToolDefinition;
}

export class UnifiedToolRegistryImpl implements UnifiedToolRegistry {
  private tools = new Map<string, RegisteredTool>();
  private stateManager: SkillStateManager;

  constructor(stateManager: SkillStateManager) {
    this.stateManager = stateManager;
  }

  /** Register all tools for a skill */
  registerTools(skillId: string, tools: SkillToolDefinition[]): void {
    for (const def of tools) {
      const toolName = def.tool.name;
      if (this.tools.has(toolName)) {
        log(
          "Tool %s already registered by skill %s, overwriting with %s",
          toolName,
          this.tools.get(toolName)!.skillId,
          skillId,
        );
      }
      this.tools.set(toolName, { skillId, def });
    }
    log("Registered %d tools for skill %s", tools.length, skillId);
  }

  /** Unregister all tools belonging to a skill */
  unregisterTools(skillId: string): void {
    let count = 0;
    for (const [name, entry] of this.tools) {
      if (entry.skillId === skillId) {
        this.tools.delete(name);
        count++;
      }
    }
    if (count > 0) {
      log("Unregistered %d tools for skill %s", count, skillId);
    }
  }

  /** List all registered tool definitions */
  listTools(): MCPTool[] {
    return Array.from(this.tools.values()).map((entry) => entry.def.tool);
  }

  /**
   * Execute a tool by name.
   * Returns undefined if the tool is not found in this registry.
   * Enforces rate limits before execution.
   */
  async executeTool(
    name: string,
    args: Record<string, unknown>,
  ): Promise<MCPToolResult | undefined> {
    const entry = this.tools.get(name);
    if (!entry) return undefined;

    // Enforce rate limits
    const tier = this.getToolTier(name);
    if (tier !== "state_only") {
      await enforceRateLimit(name);
    }

    // Build context for the tool handler
    const ctx: SkillToolContext = {
      skillId: entry.skillId,
      getState: <S>() =>
        (this.stateManager.getState<S>(entry.skillId) ?? {}) as S,
      setState: <S>(partial: Partial<S>) =>
        this.stateManager.setState(entry.skillId, partial),
      log: (message: string) =>
        log("[%s/%s] %s", entry.skillId, name, message),
    };

    return entry.def.handler(args, ctx);
  }

  /** Get the skill that owns a tool */
  getToolOwner(toolName: string): string | undefined {
    return this.tools.get(toolName)?.skillId;
  }

  /**
   * Get the rate limit tier for a tool.
   * Uses the skill definition's tier if set, otherwise falls back
   * to the existing classifyTool() for Telegram tools.
   */
  getToolTier(toolName: string): ToolTier {
    const entry = this.tools.get(toolName);
    if (entry?.def.tier) return entry.def.tier;
    // Fall back to existing rate limiter classification
    return classifyTool(toolName);
  }

  /** Check if a tool is read-only (safe for ask/plan modes) */
  isReadOnly(toolName: string): boolean {
    const entry = this.tools.get(toolName);
    if (!entry) return false;
    return entry.def.readOnly ?? false;
  }

  /** Check if a tool is registered */
  hasTool(toolName: string): boolean {
    return this.tools.has(toolName);
  }

  /** Get total number of registered tools */
  get size(): number {
    return this.tools.size;
  }

  /** Get all tool names for a specific skill */
  getToolsForSkill(skillId: string): string[] {
    const names: string[] = [];
    for (const [name, entry] of this.tools) {
      if (entry.skillId === skillId) {
        names.push(name);
      }
    }
    return names;
  }
}
