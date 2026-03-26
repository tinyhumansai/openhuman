import toolsMd from '../../../../src-tauri/ai/TOOLS.md?raw';
import type {
  SkillGroup,
  ToolCategory,
  ToolDefinition,
  ToolEnvironment,
  ToolParseResult,
  ToolsCacheEntry,
  ToolsConfig,
  ToolStatistics,
} from './types';

// GitHub URL removed - always use bundled file updated by auto-update system
const TOOLS_CACHE_KEY = 'openhuman.tools.cache';
const TOOLS_CACHE_TTL = 1000 * 60 * 30; // 30 minutes
const CACHE_VERSION = '1.0.0';

let cachedToolsConfig: ToolsConfig | null = null;

/**
 * Load TOOLS.md with caching strategy:
 * 1. Try in-memory cache
 * 2. Try localStorage cache (with TTL)
 * 3. Use bundled TOOLS.md (always fresh from auto-updates)
 */
export async function loadTools(): Promise<ToolsConfig> {
  // 1. Memory cache
  if (cachedToolsConfig) {
    return cachedToolsConfig;
  }

  // 2. Local storage cache
  try {
    const cached = localStorage.getItem(TOOLS_CACHE_KEY);
    if (cached) {
      const parsed = JSON.parse(cached) as ToolsCacheEntry;
      if (Date.now() - parsed.timestamp < TOOLS_CACHE_TTL && parsed.version === CACHE_VERSION) {
        cachedToolsConfig = parsed.config;
        return parsed.config;
      }
    }
  } catch {
    // Ignore cache errors
  }

  // 3. Always use bundled TOOLS.md (updated by auto-update system)
  const raw = toolsMd;
  const isDefault = false; // Not a fallback since it's the auto-generated file

  const config = parseTools(raw, isDefault);

  // Cache the result
  cachedToolsConfig = config;
  try {
    const cacheEntry: ToolsCacheEntry = { config, timestamp: Date.now(), version: CACHE_VERSION };
    localStorage.setItem(TOOLS_CACHE_KEY, JSON.stringify(cacheEntry));
  } catch {
    // Ignore storage errors
  }

  return config;
}

/**
 * Parse TOOLS markdown into structured config
 */
export function parseTools(raw: string, isDefault: boolean): ToolsConfig {
  const parseResult = parseToolsFromMarkdown(raw);
  const skillGroups = groupToolsBySkill(parseResult.tools);
  const categories = generateCategories(skillGroups);
  const environments = parseEnvironments(raw);
  const statistics = generateStatistics(parseResult.tools, skillGroups, categories);

  return {
    raw,
    tools: parseResult.tools,
    skillGroups,
    categories,
    environments,
    statistics,
    isDefault,
    loadedAt: Date.now(),
  };
}

/**
 * Parse tools from markdown content
 */
function parseToolsFromMarkdown(raw: string): ToolParseResult {
  const tools: ToolDefinition[] = [];
  const errors: string[] = [];
  const warnings: string[] = [];

  try {
    // Extract tools sections
    const toolsSections = raw.split(/### .+ Tools/);

    for (let i = 1; i < toolsSections.length; i++) {
      const section = toolsSections[i];
      const sectionTools = parseToolsSection(section, i);

      tools.push(...sectionTools.tools);
      errors.push(...sectionTools.errors);
      warnings.push(...sectionTools.warnings);
    }
  } catch (error) {
    errors.push(
      `Failed to parse tools markdown: ${error instanceof Error ? error.message : 'Unknown error'}`
    );
  }

  return { tools, errors, warnings };
}

/**
 * Parse tools from a single section
 */
function parseToolsSection(section: string, sectionIndex: number): ToolParseResult {
  const tools: ToolDefinition[] = [];
  const errors: string[] = [];
  const warnings: string[] = [];

  // Try to determine skillId from context
  let skillId = 'unknown';
  if (section.includes('**Category**:')) {
    // Extract from description or infer from tools
    const categoryMatch = section.match(/\*\*Category\*\*:\s*(.+)/);
    if (categoryMatch) {
      skillId = inferSkillIdFromCategory(categoryMatch[1]);
    }
  }

  // Extract individual tools
  const toolBlocks = section.split(/#### /);

  for (let j = 1; j < toolBlocks.length; j++) {
    const toolBlock = toolBlocks[j];
    try {
      const tool = parseToolBlock(toolBlock, skillId);
      if (tool) {
        tools.push(tool);
      }
    } catch (error) {
      errors.push(
        `Error parsing tool in section ${sectionIndex}, block ${j}: ${error instanceof Error ? error.message : 'Unknown error'}`
      );
    }
  }

  return { tools, errors, warnings };
}

/**
 * Parse a single tool block
 */
function parseToolBlock(block: string, defaultSkillId: string): ToolDefinition | null {
  const lines = block.trim().split('\n');
  if (lines.length === 0) return null;

  const name = lines[0].trim();
  if (!name) return null;

  // Extract description
  const descriptionMatch = block.match(/\*\*Description\*\*:\s*(.+)/);
  const description = descriptionMatch?.[1]?.trim() || 'No description available';

  // Extract parameters and build input schema (tool blocks use **Parameters**: not ## headings)
  const parametersSection = extractParametersSectionFromToolBlock(block);
  const inputSchema = parseParametersToSchema(parametersSection);

  // Try to determine actual skillId from tool name or description
  const skillId = inferSkillIdFromTool(name, description, defaultSkillId);

  return { skillId, name, description, inputSchema };
}

/**
 * Parse parameters section to JSON Schema
 */
function parseParametersToSchema(parametersText: string): ToolDefinition['inputSchema'] {
  const schema: ToolDefinition['inputSchema'] = { type: 'object', properties: {}, required: [] };

  if (!parametersText || parametersText.includes('*None*')) {
    return schema;
  }

  const paramLines = parametersText.split('\n').filter(line => line.trim().startsWith('- **'));

  for (const line of paramLines) {
    const match = line.match(/- \*\*(.+?)\*\* \((.+?)\)(\s*\*\*\(required\)\*\*)?\s*:\s*(.+)/);
    if (match) {
      const [, paramName, paramType, isRequired, paramDescription] = match;

      schema.properties[paramName] = {
        type: paramType === 'any' ? 'string' : paramType,
        description: paramDescription.trim(),
      };

      if (isRequired) {
        schema.required?.push(paramName);
      }
    }
  }

  return schema;
}

/**
 * Group tools by skill
 */
function groupToolsBySkill(tools: ToolDefinition[]): Record<string, SkillGroup> {
  const groups: Record<string, SkillGroup> = {};

  for (const tool of tools) {
    const skillId = tool.skillId;

    if (!groups[skillId]) {
      groups[skillId] = {
        skillId,
        name: formatSkillName(skillId),
        category: categorizeSkill(skillId),
        tools: [],
      };
    }

    groups[skillId].tools.push(tool);
  }

  return groups;
}

/**
 * Generate tool categories
 */
function generateCategories(skillGroups: Record<string, SkillGroup>): Record<string, ToolCategory> {
  const categories: Record<string, ToolCategory> = {};

  // Initialize predefined categories
  const predefinedCategories = {
    communication: {
      id: 'communication',
      name: 'Communication',
      description: 'Tools for messaging, email, and social interaction',
      skills: ['telegram', 'gmail', 'discord', 'slack'],
    },
    productivity: {
      id: 'productivity',
      name: 'Productivity',
      description: 'Tools for task management, note-taking, and organization',
      skills: ['notion', 'todoist', 'calendar', 'trello'],
    },
    automation: {
      id: 'automation',
      name: 'Automation',
      description: 'Tools for workflow automation and task scheduling',
      skills: ['zapier', 'ifttt', 'scheduler', 'webhook'],
    },
    data: {
      id: 'data',
      name: 'Data & Analytics',
      description: 'Tools for data processing, analysis, and storage',
      skills: ['database', 'csv', 'json', 'analytics'],
    },
    media: {
      id: 'media',
      name: 'Media & Content',
      description: 'Tools for image, video, and content processing',
      skills: ['image', 'video', 'audio', 'pdf'],
    },
    utility: {
      id: 'utility',
      name: 'Utilities',
      description: 'General-purpose utility tools and helpers',
      skills: ['file', 'text', 'crypto', 'converter'],
    },
  };

  // Initialize categories with actual skill data
  for (const [categoryId, categoryConfig] of Object.entries(predefinedCategories)) {
    const skillsInCategory = Object.values(skillGroups).filter(
      group => group.category === categoryId
    );

    categories[categoryId] = {
      ...categoryConfig,
      toolCount: skillsInCategory.reduce((sum, group) => sum + group.tools.length, 0),
    };
  }

  return categories;
}

/**
 * Parse environments from markdown
 */
function parseEnvironments(raw: string): Record<string, ToolEnvironment> {
  const environments: Record<string, ToolEnvironment> = {};

  // Extract environment sections
  const envSection = extractSection(raw, 'Environment Configuration');
  if (!envSection) {
    // Return default environments if not found in markdown
    return getDefaultEnvironments();
  }

  const envBlocks = envSection.split(/### (.+) Environment/);

  for (let i = 1; i < envBlocks.length; i += 2) {
    const envName = envBlocks[i];
    const envContent = envBlocks[i + 1] || '';

    const envId = envName.toLowerCase().replace(/[^a-z0-9]/g, '');

    environments[envId] = {
      id: envId,
      name: envName,
      description: extractFirstLine(envContent),
      accessLevel: extractListItem(envContent, 'Access Level'),
      rateLimits: extractListItem(envContent, 'Rate Limits'),
      authentication: extractListItem(envContent, 'Authentication'),
      logging: extractListItem(envContent, 'Logging'),
    };
  }

  return environments;
}

/**
 * Generate statistics
 */
function generateStatistics(
  tools: ToolDefinition[],
  skillGroups: Record<string, SkillGroup>,
  categories: Record<string, ToolCategory>
): ToolStatistics {
  const toolsByCategory: Record<string, number> = {};
  const skillsByCategory: Record<string, string[]> = {};

  for (const category of Object.keys(categories)) {
    toolsByCategory[category] = 0;
    skillsByCategory[category] = [];
  }

  for (const group of Object.values(skillGroups)) {
    const category = group.category;
    toolsByCategory[category] += group.tools.length;
    skillsByCategory[category].push(group.skillId);
  }

  return {
    totalTools: tools.length,
    activeSkills: Object.keys(skillGroups).length,
    categoriesCount: Object.keys(categories).length,
    toolsByCategory,
    skillsByCategory,
  };
}

/**
 * Utility functions
 */
/** Parameters under a tool block use `**Parameters**:` (see OpenClaw TOOLS.md format). */
function extractParametersSectionFromToolBlock(block: string): string {
  const m = block.match(/\*\*Parameters\*\*:\s*\n([\s\S]*)/i);
  return m?.[1]?.trim() ?? '';
}

function extractSection(raw: string, heading: string): string {
  const regex = new RegExp(`## ${heading}\\s*\\n([\\s\\S]*?)(?=\\n## |$)`, 'i');
  const match = raw.match(regex);
  return match?.[1]?.trim() ?? '';
}

function extractFirstLine(text: string): string {
  const lines = text.split('\n').filter(line => line.trim());
  return lines[0]?.trim() || '';
}

function extractListItem(text: string, itemName: string): string {
  const regex = new RegExp(`- \\*\\*${itemName}\\*\\*:\\s*(.+)`, 'i');
  const match = text.match(regex);
  return match?.[1]?.trim() || '';
}

function formatSkillName(skillId: string): string {
  return skillId
    .split(/[-_]/)
    .map(word => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
}

function categorizeSkill(skillId: string): string {
  const categoryMap: Record<string, string> = {
    telegram: 'communication',
    gmail: 'communication',
    discord: 'communication',
    slack: 'communication',
    notion: 'productivity',
    todoist: 'productivity',
    calendar: 'productivity',
    trello: 'productivity',
    zapier: 'automation',
    ifttt: 'automation',
    scheduler: 'automation',
    webhook: 'automation',
    database: 'data',
    csv: 'data',
    json: 'data',
    analytics: 'data',
    image: 'media',
    video: 'media',
    audio: 'media',
    pdf: 'media',
  };

  for (const [skill, category] of Object.entries(categoryMap)) {
    if (skillId.includes(skill)) {
      return category;
    }
  }

  return 'utility';
}

function inferSkillIdFromCategory(category: string): string {
  const categorySkillMap: Record<string, string> = {
    Communication: 'telegram',
    Productivity: 'notion',
    Automation: 'zapier',
    'Data & Analytics': 'database',
    'Media & Content': 'image',
  };

  return categorySkillMap[category] || 'unknown';
}

function inferSkillIdFromTool(name: string, description: string, defaultSkillId: string): string {
  const text = `${name} ${description}`.toLowerCase();

  const skillKeywords: Record<string, string[]> = {
    telegram: ['telegram', 'chat', 'message'],
    gmail: ['gmail', 'email', 'mail'],
    notion: ['notion', 'page', 'database'],
    discord: ['discord'],
    slack: ['slack'],
    todoist: ['todoist', 'task'],
    calendar: ['calendar', 'event'],
    trello: ['trello', 'board'],
    zapier: ['zapier'],
    ifttt: ['ifttt'],
  };

  for (const [skillId, keywords] of Object.entries(skillKeywords)) {
    if (keywords.some(keyword => text.includes(keyword))) {
      return skillId;
    }
  }

  return defaultSkillId;
}

function getDefaultEnvironments(): Record<string, ToolEnvironment> {
  return {
    development: {
      id: 'development',
      name: 'Development',
      description: 'Local development environment with full access',
      accessLevel: 'Full access to all tools',
      rateLimits: 'Relaxed for testing',
      authentication: 'Development credentials',
      logging: 'Verbose logging enabled',
    },
    production: {
      id: 'production',
      name: 'Production',
      description: 'Production environment with security restrictions',
      accessLevel: 'Production-safe tools only',
      rateLimits: 'Standard API limits enforced',
      authentication: 'Production credentials required',
      logging: 'Essential logs only',
    },
    testing: {
      id: 'testing',
      name: 'Testing',
      description: 'Testing environment for automated validation',
      accessLevel: 'Safe tools for automated testing',
      rateLimits: 'Testing-specific limits',
      authentication: 'Test credentials',
      logging: 'Test execution logs',
    },
  };
}

/**
 * Clear tools cache (useful for testing or manual refresh)
 */
export function clearToolsCache(): void {
  cachedToolsConfig = null;
  try {
    localStorage.removeItem(TOOLS_CACHE_KEY);
  } catch {
    // Ignore storage errors
  }
}
