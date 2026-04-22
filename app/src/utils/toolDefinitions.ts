export interface ToolDefinition {
  id: string;
  displayName: string;
  description: string;
  category: ToolCategory;
  defaultEnabled: boolean;
  rustToolNames: string[];
}

export type ToolCategory = 'System' | 'Files' | 'Vision' | 'Web' | 'Memory' | 'Automation';

export const TOOL_CATEGORIES: ToolCategory[] = [
  'System',
  'Files',
  'Vision',
  'Web',
  'Memory',
  'Automation',
];

export const TOOL_CATALOG: ToolDefinition[] = [
  // System
  {
    id: 'shell',
    displayName: 'Shell Commands',
    description: 'Execute shell commands on your machine.',
    category: 'System',
    defaultEnabled: true,
    rustToolNames: ['shell'],
  },
  {
    id: 'git_operations',
    displayName: 'Git Operations',
    description: 'Run git commands in your workspace.',
    category: 'System',
    defaultEnabled: true,
    rustToolNames: ['git_operations'],
  },

  // Files
  {
    id: 'file_read',
    displayName: 'Read Files',
    description: 'Read file contents from disk.',
    category: 'Files',
    defaultEnabled: true,
    rustToolNames: ['file_read', 'read_diff', 'csv_export'],
  },
  {
    id: 'file_write',
    displayName: 'Write Files',
    description: 'Create or modify files on disk.',
    category: 'Files',
    defaultEnabled: true,
    rustToolNames: ['file_write', 'update_memory_md'],
  },

  // Vision
  {
    id: 'screenshot',
    displayName: 'Screenshot',
    description: 'Capture screenshots of your screen.',
    category: 'Vision',
    defaultEnabled: true,
    rustToolNames: ['screenshot'],
  },
  {
    id: 'image_info',
    displayName: 'Image Analysis',
    description: 'Inspect and analyse image files.',
    category: 'Vision',
    defaultEnabled: true,
    rustToolNames: ['image_info'],
  },

  // Web
  {
    id: 'browser_open',
    displayName: 'Open Browser',
    description: 'Open URLs in your web browser.',
    category: 'Web',
    defaultEnabled: false,
    rustToolNames: ['browser_open'],
  },
  {
    id: 'browser',
    displayName: 'Browser Automation',
    description: 'Automate browser interactions.',
    category: 'Web',
    defaultEnabled: false,
    rustToolNames: ['browser'],
  },
  {
    id: 'http_request',
    displayName: 'HTTP Requests',
    description: 'Make HTTP/HTTPS requests to APIs.',
    category: 'Web',
    defaultEnabled: false,
    rustToolNames: ['http_request'],
  },
  {
    id: 'web_search',
    displayName: 'Web Search',
    description: 'Search the web for information.',
    category: 'Web',
    defaultEnabled: true,
    rustToolNames: ['web_search_tool'],
  },

  // Memory
  {
    id: 'memory_store',
    displayName: 'Store Memory',
    description: 'Save information for later recall.',
    category: 'Memory',
    defaultEnabled: true,
    rustToolNames: ['memory_store'],
  },
  {
    id: 'memory_recall',
    displayName: 'Recall Memory',
    description: 'Retrieve previously stored information.',
    category: 'Memory',
    defaultEnabled: true,
    rustToolNames: ['memory_recall'],
  },
  {
    id: 'memory_forget',
    displayName: 'Forget Memory',
    description: 'Remove stored information.',
    category: 'Memory',
    defaultEnabled: true,
    rustToolNames: ['memory_forget'],
  },

  // Automation
  {
    id: 'cron',
    displayName: 'Scheduled Tasks',
    description: 'Create and manage recurring tasks.',
    category: 'Automation',
    defaultEnabled: true,
    rustToolNames: ['cron_add', 'cron_list', 'cron_remove', 'cron_update', 'cron_run', 'cron_runs'],
  },
  {
    id: 'schedule',
    displayName: 'Remote Schedules',
    description: 'Schedule remote agent executions.',
    category: 'Automation',
    defaultEnabled: true,
    rustToolNames: ['schedule'],
  },
];

export const CATEGORY_DESCRIPTIONS: Record<ToolCategory, string> = {
  System: 'Shell access and version control',
  Files: 'Read and write files on disk',
  Vision: 'Screen capture and image analysis',
  Web: 'Browser, HTTP, and web search',
  Memory: 'Persistent recall for the AI',
  Automation: 'Cron jobs and scheduled tasks',
};

export function getToolsByCategory(): Record<ToolCategory, ToolDefinition[]> {
  const grouped = {} as Record<ToolCategory, ToolDefinition[]>;
  for (const cat of TOOL_CATEGORIES) grouped[cat] = [];
  for (const tool of TOOL_CATALOG) grouped[tool.category].push(tool);
  return grouped;
}

export function getDefaultEnabledTools(): string[] {
  return TOOL_CATALOG.filter(t => t.defaultEnabled).map(t => t.id);
}

/**
 * Expands UI-level tool toggle IDs into the Rust tool names they control.
 * Tools not present in the catalog fall back to [id] so unknown IDs are passed through.
 */
export function getEnabledRustToolNames(enabledIds: string[]): string[] {
  const idToRustNames = new Map(TOOL_CATALOG.map(t => [t.id, t.rustToolNames]));
  const result: string[] = [];
  for (const id of enabledIds) {
    const rustNames = idToRustNames.get(id);
    if (rustNames) {
      result.push(...rustNames);
    } else {
      result.push(id);
    }
  }
  return result;
}
