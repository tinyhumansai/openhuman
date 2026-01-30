/**
 * Skills Registry — Central registry for extra tools
 *
 * Manages skill registration, lookup, and the `use_extra_tool` meta-tool
 * that allows the AI to load additional tool sets on demand.
 */

import type { MCPTool, MCPToolResult } from "../types";
import type { ExtraTool, ExtraToolName, SkillToolExecutor } from "./types";
import { BULK_EXTRA_TOOL, executeBulkTool } from "./bulk";
import { REMINDERS_EXTRA_TOOL, executeRemindersTool } from "./reminders";

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

const EXTRA_TOOLS_REGISTRY: Record<ExtraToolName, ExtraTool> = {
  bulk: BULK_EXTRA_TOOL,
  reminders: REMINDERS_EXTRA_TOOL,
};

const EXTRA_TOOL_EXECUTORS: Record<ExtraToolName, SkillToolExecutor> = {
  bulk: executeBulkTool,
  reminders: executeRemindersTool,
};

/** Track which extra tools have been loaded in the current session */
const loadedExtraTools = new Set<ExtraToolName>();

// ---------------------------------------------------------------------------
// useExtraTool meta-tool definition
// ---------------------------------------------------------------------------

export const useExtraToolDefinition: MCPTool = {
  name: "use_extra_tool",
  description:
    "Load an additional set of tools (a 'skill') into the current session. " +
    "Available skills: " +
    Object.values(EXTRA_TOOLS_REGISTRY)
      .map((s) => `"${s.name}" — ${s.description}`)
      .join("; "),
  inputSchema: {
    type: "object",
    properties: {
      extra_tool: {
        type: "string",
        description: `Name of the extra tool set to load. Options: ${Object.keys(EXTRA_TOOLS_REGISTRY).join(", ")}`,
        enum: Object.keys(EXTRA_TOOLS_REGISTRY),
      },
    },
    required: ["extra_tool"],
  },
};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Execute the `use_extra_tool` meta-tool.
 * Returns a description of the loaded tools + context prompt.
 */
export function executeUseExtraTool(
  args: Record<string, unknown>,
): MCPToolResult {
  const name = args.extra_tool as string;

  if (!isExtraToolName(name)) {
    return {
      content: [
        {
          type: "text",
          text: `Unknown extra tool: "${name}". Available: ${Object.keys(EXTRA_TOOLS_REGISTRY).join(", ")}`,
        },
      ],
      isError: true,
    };
  }

  const extraTool = EXTRA_TOOLS_REGISTRY[name];
  loadedExtraTools.add(name);

  const toolNames = extraTool.tools.map((t) => t.name).join(", ");

  return {
    content: [
      {
        type: "text",
        text:
          `Extra tool "${name}" loaded. Available tools: ${toolNames}\n\n` +
          extraTool.contextPrompt,
      },
    ],
    isError: false,
  };
}

/**
 * Get the ExtraTool registration by tool name (searches all skills).
 * Returns the skill name that owns the given tool, or undefined.
 */
export function getToolOwnerSkill(toolName: string): ExtraToolName | undefined {
  for (const [skillName, skill] of Object.entries(EXTRA_TOOLS_REGISTRY)) {
    if (skill.tools.some((t) => t.name === toolName)) {
      return skillName as ExtraToolName;
    }
  }
  return undefined;
}

/**
 * Execute an extra tool by name. Automatically finds the correct skill executor.
 */
export async function executeExtraToolIfExists(
  toolName: string,
  args: Record<string, unknown>,
): Promise<MCPToolResult | undefined> {
  const skillName = getToolOwnerSkill(toolName);
  if (!skillName) return undefined;

  const executor = EXTRA_TOOL_EXECUTORS[skillName];
  if (!executor) return undefined;

  return executor(toolName, args);
}

/**
 * Get all extra tool definitions (flattened from all skills).
 * Used for MCP bridge tool listing.
 */
export function getAllExtraTools(): MCPTool[] {
  return Object.values(EXTRA_TOOLS_REGISTRY).flatMap((skill) => skill.tools);
}

/**
 * Get the loaded extra tool names in the current session.
 */
export function getLoadedExtraTools(): ReadonlySet<ExtraToolName> {
  return loadedExtraTools;
}

/**
 * Check if a tool name belongs to any extra tool set.
 */
export function isExtraToolByName(toolName: string): boolean {
  return getToolOwnerSkill(toolName) !== undefined;
}

/**
 * Check if a tool is read-only (safe for ask/plan modes).
 */
export function isExtraToolReadOnly(toolName: string): boolean {
  const skillName = getToolOwnerSkill(toolName);
  if (!skillName) return false;
  return EXTRA_TOOLS_REGISTRY[skillName].readOnlyTools?.includes(toolName) ?? false;
}

/**
 * Get the context prompt for a loaded skill.
 */
export function getSkillContextPrompt(name: ExtraToolName): string | undefined {
  return EXTRA_TOOLS_REGISTRY[name]?.contextPrompt;
}

/**
 * Reset loaded state (for testing or session reset).
 */
export function resetLoadedExtraTools(): void {
  loadedExtraTools.clear();
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function isExtraToolName(name: string): name is ExtraToolName {
  return name in EXTRA_TOOLS_REGISTRY;
}

// Re-export types
export type { ExtraTool, ExtraToolName, SkillToolExecutor } from "./types";
