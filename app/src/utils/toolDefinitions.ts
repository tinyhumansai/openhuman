export interface ToolDefinition {
  id: string;
  displayName: string;
  description: string;
  category: ToolCategory;
  defaultEnabled: boolean;
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
  },
  {
    id: 'git_operations',
    displayName: 'Git Operations',
    description: 'Run git commands in your workspace.',
    category: 'System',
    defaultEnabled: true,
  },

  // Files
  {
    id: 'file_read',
    displayName: 'Read Files',
    description: 'Read file contents from disk.',
    category: 'Files',
    defaultEnabled: true,
  },
  {
    id: 'file_write',
    displayName: 'Write Files',
    description: 'Create or modify files on disk.',
    category: 'Files',
    defaultEnabled: true,
  },

  // Vision
  {
    id: 'screenshot',
    displayName: 'Screenshot',
    description: 'Capture screenshots of your screen.',
    category: 'Vision',
    defaultEnabled: true,
  },
  {
    id: 'image_info',
    displayName: 'Image Analysis',
    description: 'Inspect and analyse image files.',
    category: 'Vision',
    defaultEnabled: true,
  },

  // Web
  {
    id: 'browser_open',
    displayName: 'Open Browser',
    description: 'Open URLs in your web browser.',
    category: 'Web',
    defaultEnabled: false,
  },
  {
    id: 'browser',
    displayName: 'Browser Automation',
    description: 'Automate browser interactions.',
    category: 'Web',
    defaultEnabled: false,
  },
  {
    id: 'http_request',
    displayName: 'HTTP Requests',
    description: 'Make HTTP/HTTPS requests to APIs.',
    category: 'Web',
    defaultEnabled: false,
  },
  {
    id: 'web_search',
    displayName: 'Web Search',
    description: 'Search the web for information.',
    category: 'Web',
    defaultEnabled: true,
  },

  // Memory
  {
    id: 'memory_store',
    displayName: 'Store Memory',
    description: 'Save information for later recall.',
    category: 'Memory',
    defaultEnabled: true,
  },
  {
    id: 'memory_recall',
    displayName: 'Recall Memory',
    description: 'Retrieve previously stored information.',
    category: 'Memory',
    defaultEnabled: true,
  },
  {
    id: 'memory_forget',
    displayName: 'Forget Memory',
    description: 'Remove stored information.',
    category: 'Memory',
    defaultEnabled: true,
  },

  // Automation
  {
    id: 'cron',
    displayName: 'Scheduled Tasks',
    description: 'Create and manage recurring tasks.',
    category: 'Automation',
    defaultEnabled: true,
  },
  {
    id: 'schedule',
    displayName: 'Remote Schedules',
    description: 'Schedule remote agent executions.',
    category: 'Automation',
    defaultEnabled: true,
  },
];

export function getToolsByCategory(): Record<ToolCategory, ToolDefinition[]> {
  const grouped = {} as Record<ToolCategory, ToolDefinition[]>;
  for (const cat of TOOL_CATEGORIES) grouped[cat] = [];
  for (const tool of TOOL_CATALOG) grouped[tool.category].push(tool);
  return grouped;
}

export function getDefaultEnabledTools(): string[] {
  return TOOL_CATALOG.filter(t => t.defaultEnabled).map(t => t.id);
}
