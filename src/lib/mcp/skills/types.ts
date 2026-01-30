/**
 * Skills / Extra Tools System — Type Definitions
 *
 * Skills are dynamic tool extensions loaded on demand via the `useExtraTool`
 * meta-tool. Each skill provides a set of MCP tools and a context prompt
 * injected into the AI system when loaded.
 */

import type { MCPTool, MCPToolResult } from "../types";

/** Names of available extra tool sets */
export type ExtraToolName = "bulk" | "reminders";

/**
 * An extra tool set that can be loaded on demand.
 */
export interface ExtraTool {
  /** Unique skill name */
  name: ExtraToolName;
  /** Human-readable description shown to the AI before loading */
  description: string;
  /** MCP tool definitions provided by this skill */
  tools: MCPTool[];
  /** Context prompt injected into the AI system when this skill is loaded */
  contextPrompt: string;
  /** Tool names that are safe for read-only / ask modes */
  readOnlyTools?: string[];
}

/**
 * Result of executing a skill tool.
 */
export interface SkillToolResult {
  success: boolean;
  data?: unknown;
  error?: string;
  affectedChatIds?: string[];
}

/**
 * Executor function for a skill's tools.
 */
export type SkillToolExecutor = (
  toolName: string,
  args: Record<string, unknown>,
) => Promise<MCPToolResult>;
