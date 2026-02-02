import type { MemoryManager } from '../memory/manager';
import type { AITool, ToolResult } from './registry';

/**
 * Create the memory_read tool.
 * Reads specific memory files or line ranges.
 */
export function createMemoryReadTool(memoryManager: MemoryManager): AITool {
  return {
    definition: {
      name: 'memory_read',
      description:
        'Read a specific memory file or specific lines from a memory file. Use after memory_search to get full context of a result.',
      parameters: {
        type: 'object',
        properties: {
          path: {
            type: 'string',
            description:
              "Relative path to the memory file (e.g., 'memory.md', 'memory/2024-01-15.md', 'memory/preferences.md').",
          },
          startLine: {
            type: 'number',
            description: 'Start line number (0-indexed). If omitted, reads from the beginning.',
          },
          endLine: {
            type: 'number',
            description: 'End line number (0-indexed, inclusive). If omitted, reads to the end.',
          },
        },
        required: ['path'],
      },
    },

    async execute(args: Record<string, unknown>): Promise<ToolResult> {
      const path = String(args.path || '');
      if (!path) {
        return { content: 'Error: path is required', isError: true };
      }

      try {
        const content = await memoryManager.readFile(path);
        const lines = content.split('\n');

        const startLine = args.startLine !== undefined ? Number(args.startLine) : 0;
        const endLine =
          args.endLine !== undefined
            ? Math.min(Number(args.endLine), lines.length - 1)
            : lines.length - 1;

        const selectedLines = lines.slice(startLine, endLine + 1);
        const result = selectedLines.join('\n');

        return {
          content: `**File**: ${path} (lines ${startLine}-${endLine} of ${lines.length})\n\n${result}`,
        };
      } catch {
        return { content: `File not found or unreadable: ${path}`, isError: true };
      }
    },
  };
}
