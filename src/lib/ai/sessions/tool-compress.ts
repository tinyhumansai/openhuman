import type { Message, MessageContent } from '../providers/interface';
import type { ToolCaptureConfig } from './types';

export type { ToolCaptureConfig };

export const DEFAULT_TOOL_CAPTURE_CONFIG: ToolCaptureConfig = {
  skipTools: ['memory_read', 'memory_search'],
  captureTools: ['memory_write', 'web_search'],
};

function getRelativePath(filePath: string | undefined): string {
  if (!filePath) return 'unknown';
  const parts = filePath.split('/');
  return parts.slice(-2).join('/');
}

function truncate(str: string | undefined, maxLen = 50): string {
  if (!str) return '';
  if (str.length <= maxLen) return str;
  return `${str.slice(0, maxLen)}...`;
}

/**
 * Compress a tool observation into a human-readable one-liner.
 * Ported from claude-supermemory's compress.js, adapted for AlphaHuman tools.
 */
export function compressObservation(
  toolName: string,
  toolInput: Record<string, unknown>,
  toolResponse?: string
): string {
  const input = toolInput || {};

  switch (toolName) {
    case 'memory_write': {
      const path = String(input.path || 'unknown');
      const mode = String(input.mode || 'append');
      const contentLen = String(input.content || '').length;
      return `${mode === 'overwrite' ? 'Wrote' : 'Appended'} to ${path} (${contentLen} chars)`;
    }
    case 'memory_search': {
      const query = truncate(String(input.query || ''), 60);
      const hasResults = toolResponse && !toolResponse.includes('No matching');
      return `Searched memory: "${query}"${hasResults ? '' : ' [no results]'}`;
    }
    case 'memory_read': {
      const path = String(input.path || 'unknown');
      const lines = input.startLine != null ? ` (lines ${input.startLine}-${input.endLine})` : '';
      return `Read ${path}${lines}`;
    }
    case 'web_search': {
      const query = truncate(String(input.query || ''), 60);
      return `Searched web: "${query}"`;
    }
    case 'Edit': {
      const file = getRelativePath(input.file_path as string | undefined);
      const oldSnippet = truncate(input.old_string as string | undefined, 30);
      const newSnippet = truncate(input.new_string as string | undefined, 30);
      if (input.replace_all) return `Replaced all "${oldSnippet}" with "${newSnippet}" in ${file}`;
      return `Edited ${file}: "${oldSnippet}" -> "${newSnippet}"`;
    }
    case 'Write': {
      const file = getRelativePath(input.file_path as string | undefined);
      const contentLen = String(input.content || '').length;
      return `Created ${file} (${contentLen} chars)`;
    }
    case 'Bash': {
      const cmd = truncate(input.command as string | undefined, 80);
      const desc = input.description ? ` - ${truncate(input.description as string, 40)}` : '';
      const failed = toolResponse?.includes('[FAILED]') || toolResponse?.includes('error');
      return `Ran: ${cmd}${desc}${failed ? ' [FAILED]' : ''}`;
    }
    case 'Read': {
      const file = getRelativePath(input.file_path as string | undefined);
      const lines = input.limit ? ` (${input.limit} lines)` : '';
      return `Read ${file}${lines}`;
    }
    case 'Glob': {
      const pattern = String(input.pattern || '*');
      const path = input.path ? ` in ${getRelativePath(input.path as string)}` : '';
      return `Glob: ${pattern}${path}`;
    }
    case 'Grep': {
      const pattern = truncate(input.pattern as string | undefined, 40);
      const path = input.path ? ` in ${getRelativePath(input.path as string)}` : '';
      return `Grep: "${pattern}"${path}`;
    }
    case 'WebFetch': {
      const url = truncate(input.url as string | undefined, 60);
      return `Fetched: ${url}`;
    }
    case 'WebSearch': {
      const query = truncate(input.query as string | undefined, 60);
      return `Searched web: "${query}"`;
    }
    case 'Task': {
      const desc =
        (input.description as string) ||
        truncate(input.prompt as string | undefined, 60) ||
        'subtask';
      const agent = String(input.subagent_type || 'agent');
      return `Spawned ${agent}: ${desc}`;
    }
    default:
      return `Used ${toolName}`;
  }
}

/**
 * Walk a message array, compress tool_use/tool_result pairs.
 *
 * - Skipped tools: both tool_use and tool_result are dropped entirely
 * - Captured/other tools: tool_result content is replaced with a compressed one-liner
 */
export function compressMessagesForSummary(
  messages: Message[],
  config: ToolCaptureConfig = DEFAULT_TOOL_CAPTURE_CONFIG
): Message[] {
  // Build a map of tool_use id -> tool name for matching tool_results
  const toolUseMap = new Map<string, { name: string; input: Record<string, unknown> }>();

  for (const msg of messages) {
    for (const block of msg.content) {
      if (block.type === 'tool_use') {
        toolUseMap.set(block.id, { name: block.name, input: block.input });
      }
    }
  }

  const compressed: Message[] = [];

  for (const msg of messages) {
    const newContent: MessageContent[] = [];

    for (const block of msg.content) {
      if (block.type === 'tool_use') {
        // Skip tool_use blocks for skipped tools
        if (config.skipTools.includes(block.name)) continue;
        // Keep tool_use blocks but they're small anyway
        newContent.push(block);
      } else if (block.type === 'tool_result') {
        const toolInfo = toolUseMap.get(block.toolUseId);
        if (toolInfo && config.skipTools.includes(toolInfo.name)) {
          // Drop results from skipped tools entirely
          continue;
        }
        if (toolInfo) {
          // Compress the result into a one-liner
          const summary = compressObservation(toolInfo.name, toolInfo.input, block.content);
          newContent.push({
            type: 'tool_result',
            toolUseId: block.toolUseId,
            content: summary,
            isError: block.isError,
          });
        } else {
          // Unknown tool_use — keep as-is but truncate
          newContent.push({
            type: 'tool_result',
            toolUseId: block.toolUseId,
            content: truncate(block.content, 200),
            isError: block.isError,
          });
        }
      } else {
        // Text blocks — keep as-is
        newContent.push(block);
      }
    }

    // Only include messages that still have content
    if (newContent.length > 0) {
      compressed.push({ ...msg, content: newContent });
    }
  }

  return compressed;
}
