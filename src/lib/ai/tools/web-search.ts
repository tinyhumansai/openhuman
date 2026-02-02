import type { AITool, ToolResult } from './registry';

/**
 * Web search configuration.
 */
export interface WebSearchConfig {
  /** Search API endpoint */
  endpoint?: string;
  /** API key for search service */
  apiKey?: string;
}

/**
 * Create the web_search tool.
 * Searches the web for current information.
 */
export function createWebSearchTool(config: WebSearchConfig = {}): AITool {
  return {
    definition: {
      name: 'web_search',
      description:
        "Search the web for current information. Useful for real-time crypto prices, news, protocol updates, and on-chain data that isn't in memory.",
      parameters: {
        type: 'object',
        properties: {
          query: { type: 'string', description: 'Search query.' },
          limit: { type: 'number', description: 'Max results to return (default: 5).' },
        },
        required: ['query'],
      },
    },

    async execute(args: Record<string, unknown>): Promise<ToolResult> {
      const query = String(args.query || '');
      if (!query.trim()) {
        return { content: 'Error: query is required', isError: true };
      }

      if (!config.endpoint || !config.apiKey) {
        return {
          content:
            'Web search is not configured. Please set up a search API endpoint and key in settings.',
          isError: true,
        };
      }

      try {
        const response = await fetch(
          `${config.endpoint}?q=${encodeURIComponent(query)}&count=${Number(args.limit) || 5}`,
          { headers: { Authorization: `Bearer ${config.apiKey}`, Accept: 'application/json' } }
        );

        if (!response.ok) {
          return {
            content: `Search API error: ${response.status} ${response.statusText}`,
            isError: true,
          };
        }

        const data = await response.json();

        // Format results (generic format — adapt to specific search API)
        const results = (data.results || data.web?.results || [])
          .slice(0, Number(args.limit) || 5)
          .map(
            (
              r: { title?: string; url?: string; description?: string; snippet?: string },
              i: number
            ) =>
              `### ${i + 1}. ${r.title || 'Untitled'}\n` +
              `**URL**: ${r.url || 'N/A'}\n` +
              `${r.description || r.snippet || 'No description'}`
          )
          .join('\n\n');

        return { content: results || 'No results found.' };
      } catch (error) {
        return {
          content: `Search failed: ${error instanceof Error ? error.message : String(error)}`,
          isError: true,
        };
      }
    },
  };
}
