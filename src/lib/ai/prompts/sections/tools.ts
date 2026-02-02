import type { ToolDefinition } from '../../providers/interface';

/**
 * Build the available tools section of the system prompt.
 */
export function buildToolsSection(tools: ToolDefinition[]): string {
  if (tools.length === 0) return '';

  const parts: string[] = [];

  parts.push('## Available Tools\n');
  parts.push('You have access to the following tools:\n');

  for (const tool of tools) {
    parts.push(`- **${tool.name}**: ${tool.description}`);
  }
  parts.push('');

  parts.push('Tool usage guidelines:');
  parts.push('- Always use the most specific tool available for the task');
  parts.push('- For memory operations: search before writing to avoid duplicates');
  parts.push('- Report tool errors to the user clearly');
  parts.push('');

  return parts.join('\n');
}
