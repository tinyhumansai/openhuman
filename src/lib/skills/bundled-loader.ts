/**
 * Bundled Skill Loader — Build-Time Vite Imports
 *
 * Loads skills from the skills/ submodule using Vite dynamic imports.
 * Each skill is imported at build time and registered with the orchestrator.
 */

import type { SkillDefinition } from "./types";
import createDebug from "debug";

const log = createDebug("app:skills:bundled");

/**
 * Map of bundled skill IDs to their Vite dynamic import functions.
 * Each function returns a module with a default export of the old-format
 * skill definition (from the skills/ submodule).
 */
const BUNDLED_SKILL_MODULES: Record<
  string,
  () => Promise<{ default: BundledSkillExport }>
> = {
  "price-tracker": () =>
    import("../../../skills/skills/price-tracker/skill") as Promise<{
      default: BundledSkillExport;
    }>,
  "portfolio-analysis": () =>
    import("../../../skills/skills/portfolio-analysis/skill") as Promise<{
      default: BundledSkillExport;
    }>,
  "on-chain-lookup": () =>
    import("../../../skills/skills/on-chain-lookup/skill") as Promise<{
      default: BundledSkillExport;
    }>,
  "trading-signals": () =>
    import("../../../skills/skills/trading-signals/skill") as Promise<{
      default: BundledSkillExport;
    }>,
};

/**
 * Shape of a bundled skill's default export (from skills/ submodule).
 * This matches the @alphahuman/skill-types SkillDefinition in the submodule,
 * which differs from our unified SkillDefinition.
 */
interface BundledSkillExport {
  name: string;
  description: string;
  version: string;
  hooks?: {
    onLoad?(ctx: unknown): Promise<void>;
    onUnload?(ctx: unknown): Promise<void>;
    onSessionStart?(ctx: unknown, sessionId: string): Promise<void>;
    onSessionEnd?(ctx: unknown, sessionId: string): Promise<void>;
    onBeforeMessage?(ctx: unknown, message: string): Promise<string | void>;
    onAfterResponse?(ctx: unknown, response: string): Promise<string | void>;
    onMemoryFlush?(ctx: unknown): Promise<void>;
    onTick?(ctx: unknown): Promise<void>;
  };
  tools?: Array<{
    definition: {
      name: string;
      description: string;
      parameters: {
        type: "object";
        properties: Record<string, unknown>;
        required?: string[];
      };
    };
    execute(args: Record<string, unknown>): Promise<{ content: string; isError?: boolean }>;
  }>;
  tickInterval?: number;
}

/**
 * Load a bundled skill by ID and convert it to a unified SkillDefinition.
 * Returns undefined if the skill is not found or fails to load.
 */
export async function loadBundledSkill(
  skillId: string,
): Promise<SkillDefinition | undefined> {
  const loader = BUNDLED_SKILL_MODULES[skillId];
  if (!loader) {
    log("No bundled skill found for ID: %s", skillId);
    return undefined;
  }

  try {
    const module = await loader();
    const exported = module.default;
    return convertBundledToUnified(skillId, exported);
  } catch (error) {
    log(
      "Failed to load bundled skill %s: %s",
      skillId,
      error instanceof Error ? error.message : String(error),
    );
    return undefined;
  }
}

/** Get all available bundled skill IDs */
export function getBundledSkillIds(): string[] {
  return Object.keys(BUNDLED_SKILL_MODULES);
}

/**
 * Convert a bundled skill export (old format from submodule) to
 * the unified SkillDefinition format.
 */
function convertBundledToUnified(
  skillId: string,
  exported: BundledSkillExport,
): SkillDefinition {
  const def: SkillDefinition = {
    manifest: {
      id: skillId,
      name: exported.name,
      description: exported.description,
      version: exported.version,
      tier: "bundled",
    },
    tickInterval: exported.tickInterval,
  };

  // Convert hooks — the bundled skill hooks accept the old SkillContext,
  // but we'll pass the new SkillRuntimeContext. Since the old context is
  // a subset of the new one, this is forward-compatible.
  if (exported.hooks) {
    def.hooks = {};
    const hookNames = [
      "onLoad",
      "onUnload",
      "onSessionStart",
      "onSessionEnd",
      "onBeforeMessage",
      "onAfterResponse",
      "onMemoryFlush",
      "onTick",
    ] as const;

    for (const hookName of hookNames) {
      const hookFn = exported.hooks[hookName];
      if (hookFn) {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        (def.hooks as any)[hookName] = hookFn;
      }
    }
  }

  // Convert tools from old AITool format to SkillToolDefinition
  if (exported.tools && exported.tools.length > 0) {
    def.tools = exported.tools.map((tool) => ({
      tool: {
        name: tool.definition.name,
        description: tool.definition.description,
        inputSchema: {
          type: "object" as const,
          properties: tool.definition.parameters.properties,
          required: tool.definition.parameters.required,
        },
      },
      handler: async (args) => {
        const result = await tool.execute(args);
        return {
          content: [{ type: "text" as const, text: result.content }],
          isError: result.isError ?? false,
        };
      },
      // Bundled skill tools default to api_read tier
      tier: "api_read" as const,
      readOnly: false,
    }));
  }

  return def;
}
