import type { ConstitutionConfig } from '../constitution/types';
import { sanitizeForMemory, validateMemoryContent } from '../constitution/validator';
import type { MemoryManager } from '../memory/manager';
import type { AITool, ToolResult } from './registry';

/**
 * Create the memory_write tool.
 * Writes or appends to memory files, with constitutional validation.
 */
export function createMemoryWriteTool(
  memoryManager: MemoryManager,
  constitution: ConstitutionConfig
): AITool {
  return {
    definition: {
      name: 'memory_write',
      description:
        'Write or append content to a memory file. Validates content against constitutional rules (no secrets, proper tagging). Use for storing durable facts, decisions, preferences, and notes.',
      parameters: {
        type: 'object',
        properties: {
          path: {
            type: 'string',
            description:
              "Relative path to write (e.g., 'memory.md', 'memory/preferences.md', 'memory/portfolio.md').",
          },
          content: { type: 'string', description: 'Content to write or append.' },
          mode: {
            type: 'string',
            enum: ['append', 'overwrite'],
            description:
              "Write mode: 'append' adds to existing content (default), 'overwrite' replaces the file.",
          },
        },
        required: ['path', 'content'],
      },
    },

    async execute(args: Record<string, unknown>): Promise<ToolResult> {
      const path = String(args.path || '');
      const content = String(args.content || '');
      const mode = String(args.mode || 'append');

      if (!path) {
        return { content: 'Error: path is required', isError: true };
      }
      if (!content.trim()) {
        return { content: 'Error: content is required', isError: true };
      }

      // Validate against constitution
      const validation = validateMemoryContent(content, constitution);
      if (!validation.valid) {
        const violations = validation.violations
          .map(v => `- [${v.severity}] ${v.message}`)
          .join('\n');
        return {
          content: `Constitutional violation detected. Content not written.\n\n${violations}`,
          isError: true,
        };
      }

      // Sanitize content (redact any detected secrets as a safety net)
      const sanitized = sanitizeForMemory(content);

      try {
        if (mode === 'overwrite') {
          await memoryManager.writeFile(path, sanitized);
        } else {
          await memoryManager.appendToFile(path, sanitized);
        }

        return {
          content: `Successfully ${mode === 'overwrite' ? 'wrote' : 'appended'} to ${path}`,
        };
      } catch (error) {
        return {
          content: `Failed to write: ${error instanceof Error ? error.message : String(error)}`,
          isError: true,
        };
      }
    },
  };
}
