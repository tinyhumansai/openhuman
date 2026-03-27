/**
 * Agent Tool Registry Types
 *
 * Minimal type definitions for agent tool registry functionality.
 * Based on ZeroClaw compatibility requirements.
 */

/**
 * Tool schema for AI provider registration (OpenAI function calling format)
 */
export interface AgentToolSchema {
  type: 'function';
  function: {
    name: string;
    description: string;
    parameters: {
      type: 'object';
      properties: Record<string, unknown>;
      required?: string[];
      additionalProperties?: boolean;
    };
  };
}

/**
 * Tool execution record
 */
export interface AgentToolExecution {
  id: string;
  toolName: string;
  skillId: string;
  arguments: string; // JSON string
  status: 'running' | 'success' | 'error';
  startTime: number;
  endTime?: number;
  executionTimeMs?: number;
  result?: string;
  errorMessage?: string;
}

/**
 * Agent tool registry service interface
 */
export interface IAgentToolRegistry {
  /**
   * Load tool schemas from the skill system
   */
  loadToolSchemas(forceReload?: boolean): Promise<AgentToolSchema[]>;

  /**
   * Execute a tool using the skill system
   */
  executeTool(
    skillId: string,
    toolName: string,
    toolArguments: string
  ): Promise<AgentToolExecution>;

  /**
   * Get a specific tool by name
   */
  getToolByName(toolName: string): AgentToolSchema | undefined;

  /**
   * Get all available tools
   */
  getAllTools(): AgentToolSchema[];
}
