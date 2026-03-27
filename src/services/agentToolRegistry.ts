/**
 * Agent Tool Registry Service
 *
 * Unified tool discovery and execution using the consolidated systems:
 * - telegram_get_tools: Get Telegram tools in OpenAI-compatible format
 * - telegram_execute_tool: Execute Telegram tools with enhanced validation
 * - Fallback to skill system for non-Telegram tools (temporary)
 */
import { invoke } from '@tauri-apps/api/core';

import type {
  AgentToolExecution,
  AgentToolSchema,
  IAgentToolRegistry,
} from '../types/agent';

// ZeroClaw format types from Rust
interface ZeroClawToolSchema {
  type: string;
  function: { name: string; description: string; parameters: Record<string, unknown> };
}

interface ZeroClawToolResult {
  success: boolean;
  output: string;
  error?: string;
  execution_time?: number;
}

export class AgentToolRegistry implements IAgentToolRegistry {
  private static instance: AgentToolRegistry;
  private toolSchemas: AgentToolSchema[] = [];
  private lastLoadTime = 0;
  private readonly CACHE_TTL = 5 * 60 * 1000; // 5 minutes

  static getInstance(): AgentToolRegistry {
    if (!this.instance) {
      this.instance = new AgentToolRegistry();
    }
    return this.instance;
  }

  /**
   * Load tool schemas from unified systems (Telegram + skill system fallback)
   */
  async loadToolSchemas(forceReload = false): Promise<AgentToolSchema[]> {
    const now = Date.now();

    // Return cached tools if still fresh
    if (!forceReload && this.toolSchemas.length > 0 && now - this.lastLoadTime < this.CACHE_TTL) {
      return this.toolSchemas;
    }

    try {
      console.log('🔧 Loading tool schemas from unified systems...');

      const allTools: AgentToolSchema[] = [];

      // Note: Telegram tools removed - no longer available
      console.log('🔧 Telegram tools not available (unified system removed)');

      // 2. Load other tools from skill system (fallback for non-Telegram)
      try {
        console.log('🔧 Loading non-Telegram tools from skill system...');
        const skillTools = await invoke<ZeroClawToolSchema[]>('runtime_get_tool_schemas');

        // Filter out telegram tools to avoid duplicates
        const nonTelegramTools = skillTools.filter(
          tool =>
            !tool.function.name.includes('telegram') &&
            !tool.function.name.includes('tg') &&
            !this.extractCategoryFromSkillId(
              this.extractSkillIdFromToolName(tool.function.name) || ''
            ).includes('Telegram')
        );

        const skillSchemas = nonTelegramTools.map(tool => ({
          type: 'function' as const,
          function: {
            name: tool.function.name,
            description: tool.function.description,
            parameters: tool.function.parameters as AgentToolSchema['function']['parameters'],
          },
        }));

        allTools.push(...skillSchemas);
        console.log(`✅ Loaded ${skillSchemas.length} non-Telegram tools from skill system`);
      } catch (error) {
        console.warn('⚠️ Failed to load tools from skill system:', error);
      }

      this.toolSchemas = allTools;
      this.lastLoadTime = now;

      console.log(`✅ Tool registry updated: ${this.toolSchemas.length} total tools available`);

      return this.toolSchemas;
    } catch (error) {
      console.error('❌ Failed to load tool schemas:', error);
      throw new Error(`Failed to load tool schemas: ${error}`);
    }
  }

  /**
   * Execute a tool using unified systems (Telegram unified or skill system fallback)
   */
  async executeTool(
    skillId: string,
    toolName: string,
    toolArguments: string
  ): Promise<AgentToolExecution> {
    const startTime = Date.now();
    const executionId = `exec_${startTime}_${Math.random().toString(36).substr(2, 9)}`;

    const execution: AgentToolExecution = {
      id: executionId,
      toolName,
      skillId,
      arguments: toolArguments,
      status: 'running',
      startTime,
    };

    console.log(`🚀 [TOOL EXECUTION START] Executing tool: ${toolName} (skillId: ${skillId})`);
    console.log(`📝 [ARGUMENTS] Raw arguments:`, {
      arguments: toolArguments,
      type: typeof toolArguments,
      length: toolArguments?.length,
      isString: typeof toolArguments === 'string',
      parsed: (() => {
        try {
          return typeof toolArguments === 'string' ? JSON.parse(toolArguments) : toolArguments;
        } catch (e) {
          return 'Failed to parse: ' + e;
        }
      })(),
    });

    try {
      // Determine if this is a Telegram tool
      const isTelegramTool =
        skillId.includes('telegram') ||
        skillId.includes('tg') ||
        toolName.includes('telegram') ||
        toolName.includes('tg') ||
        this.extractCategoryFromSkillId(skillId).includes('Telegram');

      let result: ZeroClawToolResult;

      if (isTelegramTool) {
        // Telegram tools no longer available
        console.log(`🔧 [TELEGRAM TOOL] Tool "${toolName}" not available (unified system removed)`);
        result = {
          success: false,
          output: '',
          error: 'Telegram tools are no longer available (unified system removed)',
        };
      } else {
        // Use skill system for non-Telegram tools
        const toolId = `${skillId}_${toolName}`;
        console.log(`🔧 [BEFORE INVOKE] Calling runtime_execute_tool with:`);
        console.log(`   toolId: "${toolId}"`);
        console.log(`   args: ${toolArguments}`);
        console.log(`   args type: ${typeof toolArguments}`);

        result = await invoke<ZeroClawToolResult>('runtime_execute_tool', {
          toolId,
          args: toolArguments,
        });
      }

      console.log(`🔧 [AFTER INVOKE] Tool execution result:`, result);

      execution.endTime = Date.now();
      // Use execution time from Rust if available, otherwise calculate locally
      execution.executionTimeMs = result.execution_time || execution.endTime - execution.startTime;

      if (!result.success) {
        execution.status = 'error';
        execution.errorMessage = result.error || 'Unknown error occurred';
        execution.result = execution.errorMessage;

        console.log(`❌ Tool execution failed: ${toolName} (${execution.executionTimeMs}ms)`);
        console.log(`❌ Error:`, execution.errorMessage);
      } else {
        execution.status = 'success';
        execution.result = result.output;

        console.log(`✅ Tool execution completed: ${toolName} (${execution.executionTimeMs}ms)`);
      }

      return execution;
    } catch (error) {
      execution.endTime = Date.now();
      execution.executionTimeMs = execution.endTime - execution.startTime;
      execution.status = 'error';
      execution.errorMessage = error instanceof Error ? error.message : String(error);
      execution.result = execution.errorMessage;

      console.error(`❌ Tool execution error: ${toolName}`, error);

      return execution;
    }
  }

  /**
   * Get a specific tool by name
   */
  getToolByName(toolName: string): AgentToolSchema | undefined {
    return this.toolSchemas.find(tool => tool.function.name === toolName);
  }

  /**
   * Get all available tools
   */
  getAllTools(): AgentToolSchema[] {
    return [...this.toolSchemas];
  }

  /**
   * Get tools organized by skill
   */
  getToolsBySkill(): Record<string, AgentToolSchema[]> {
    const toolsBySkill: Record<string, AgentToolSchema[]> = {};

    for (const tool of this.toolSchemas) {
      // Extract skill ID from tool name (format: skillId_toolName)
      const skillId = this.extractSkillIdFromToolName(tool.function.name) || 'unknown';

      if (!toolsBySkill[skillId]) {
        toolsBySkill[skillId] = [];
      }
      toolsBySkill[skillId].push(tool);
    }

    return toolsBySkill;
  }

  /**
   * Get tool execution statistics
   */
  getToolStats(): { totalTools: number; skillCount: number; categories: Record<string, number> } {
    const categories: Record<string, number> = {};
    const skills = new Set<string>();

    for (const tool of this.toolSchemas) {
      const skillId = this.extractSkillIdFromToolName(tool.function.name) || 'unknown';
      skills.add(skillId);

      // Categorize by skill name
      const category = this.extractCategoryFromSkillId(skillId);
      categories[category] = (categories[category] || 0) + 1;
    }

    return { totalTools: this.toolSchemas.length, skillCount: skills.size, categories };
  }

  /**
   * Clear the tool registry cache
   */
  clearCache(): void {
    this.toolSchemas = [];
    this.lastLoadTime = 0;
    console.log('🔧 Tool registry cache cleared');
  }

  // =============================================================================
  // Private Helper Methods
  // =============================================================================

  /**
   * Extract skill ID from tool name (format: skillId_toolName)
   */
  private extractSkillIdFromToolName(toolName: string): string | null {
    const underscoreIndex = toolName.lastIndexOf('_');
    if (underscoreIndex === -1) {
      return null;
    }
    return toolName.substring(0, underscoreIndex);
  }

  /**
   * Extract category name from skill ID for organization
   */
  private extractCategoryFromSkillId(skillId: string): string {
    // Common skill naming patterns
    if (skillId.includes('github') || skillId.includes('git')) return 'GitHub';
    if (skillId.includes('notion')) return 'Notion';
    if (skillId.includes('telegram') || skillId.includes('tg')) return 'Telegram';
    if (skillId.includes('email') || skillId.includes('gmail')) return 'Email';
    if (skillId.includes('calendar')) return 'Calendar';
    if (skillId.includes('slack')) return 'Slack';
    if (skillId.includes('discord')) return 'Discord';
    if (skillId.includes('twitter') || skillId.includes('x')) return 'Social';
    if (skillId.includes('file') || skillId.includes('fs')) return 'File System';
    if (skillId.includes('crypto') || skillId.includes('blockchain')) return 'Crypto';
    if (skillId.includes('ai') || skillId.includes('ml')) return 'AI/ML';

    return 'Other';
  }
}
