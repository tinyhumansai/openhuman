import type { ToolDefinition } from '../providers/interface';

/** Result from executing a tool */
export interface ToolResult {
  content: string;
  isError?: boolean;
}

/** AI tool with execute capability */
export interface AITool {
  /** Tool definition for the LLM */
  definition: ToolDefinition;
  /** Execute the tool with given arguments */
  execute(args: Record<string, unknown>): Promise<ToolResult>;
}

/**
 * Tool registry manages available AI tools.
 */
export class ToolRegistry {
  private tools = new Map<string, AITool>();

  /** Register a tool */
  register(tool: AITool): void {
    this.tools.set(tool.definition.name, tool);
  }

  /** Unregister a tool */
  unregister(name: string): void {
    this.tools.delete(name);
  }

  /** Get a tool by name */
  get(name: string): AITool | undefined {
    return this.tools.get(name);
  }

  /** Get all tool definitions for the LLM */
  getDefinitions(): ToolDefinition[] {
    return Array.from(this.tools.values()).map(t => t.definition);
  }

  /** Execute a tool by name */
  async execute(name: string, args: Record<string, unknown>): Promise<ToolResult> {
    const tool = this.tools.get(name);
    if (!tool) {
      return { content: `Unknown tool: ${name}`, isError: true };
    }
    try {
      return await tool.execute(args);
    } catch (error) {
      return {
        content: `Tool execution failed: ${error instanceof Error ? error.message : String(error)}`,
        isError: true,
      };
    }
  }

  /** Get count of registered tools */
  get size(): number {
    return this.tools.size;
  }
}
