import { formatRelativeTime } from '../memory/context-formatter';
import type { MemoryManager } from '../memory/manager';
import type { AITool, ToolResult } from './registry';

/**
 * Create the memory_search tool.
 * Searches memory files and sessions using hybrid FTS5 + vector search.
 */
export function createMemorySearchTool(memoryManager: MemoryManager): AITool {
  return {
    definition: {
      name: 'memory_search',
      description:
        'Search through memory files and past sessions for relevant information. Use before answering about prior decisions, preferences, dates, people, or todos.',
      parameters: {
        type: 'object',
        properties: {
          query: {
            type: 'string',
            description:
              "Natural language search query. Be specific about what you're looking for.",
          },
          limit: {
            type: 'number',
            description: 'Maximum number of results to return (default: 5, max: 20).',
          },
        },
        required: ['query'],
      },
    },

    async execute(args: Record<string, unknown>): Promise<ToolResult> {
      const query = String(args.query || '');
      const limit = Math.min(Number(args.limit) || 5, 20);

      if (!query.trim()) {
        return { content: 'Error: query is required', isError: true };
      }

      const results = await memoryManager.search(query);
      const limited = results.slice(0, limit);

      if (limited.length === 0) {
        return { content: 'No matching memories found for the given query.' };
      }

      const formatted = limited
        .map((r, i) => {
          const timeStr = r.updatedAt ? `, ${formatRelativeTime(r.updatedAt)}` : '';
          return (
            `### Result ${i + 1} (score: ${r.score.toFixed(3)}${timeStr})\n` +
            `**File**: ${r.path} (lines ${r.startLine}-${r.endLine})\n` +
            `**Content**:\n${r.text}`
          );
        })
        .join('\n\n---\n\n');

      return { content: `Found ${limited.length} relevant memories:\n\n${formatted}` };
    },
  };
}
