import { describe, expect, it } from 'vitest';

import type { Message } from '../../providers/interface';
import {
  compressMessagesForSummary,
  compressObservation,
  DEFAULT_TOOL_CAPTURE_CONFIG,
  type ToolCaptureConfig,
} from '../tool-compress';

describe('compressObservation', () => {
  it('should compress memory_write tool', () => {
    const result = compressObservation('memory_write', {
      path: 'memory.md',
      content: 'User prefers dark mode',
      mode: 'append',
    });
    expect(result).toBe('Appended to memory.md (22 chars)');
  });

  it('should compress memory_write with overwrite mode', () => {
    const result = compressObservation('memory_write', {
      path: 'memory.md',
      content: 'New content',
      mode: 'overwrite',
    });
    expect(result).toBe('Wrote to memory.md (11 chars)');
  });

  it('should compress memory_search with results', () => {
    const result = compressObservation(
      'memory_search',
      { query: 'dark mode preference' },
      'Found 2 relevant memories'
    );
    expect(result).toBe('Searched memory: "dark mode preference"');
  });

  it('should compress memory_search with no results', () => {
    const result = compressObservation(
      'memory_search',
      { query: 'nonexistent' },
      'No matching memories found'
    );
    expect(result).toContain('[no results]');
  });

  it('should compress memory_read', () => {
    const result = compressObservation('memory_read', { path: 'memory/preferences.md' });
    expect(result).toBe('Read memory/preferences.md');
  });

  it('should compress memory_read with line range', () => {
    const result = compressObservation('memory_read', {
      path: 'memory.md',
      startLine: 5,
      endLine: 12,
    });
    expect(result).toBe('Read memory.md (lines 5-12)');
  });

  it('should compress web_search', () => {
    const result = compressObservation('web_search', { query: 'bitcoin price today' });
    expect(result).toBe('Searched web: "bitcoin price today"');
  });

  it('should compress Edit tool', () => {
    const result = compressObservation('Edit', {
      file_path: '/Users/dev/project/src/main.ts',
      old_string: 'const foo = bar',
      new_string: 'const foo = baz',
    });
    expect(result).toBe('Edited src/main.ts: "const foo = bar" -> "const foo = baz"');
  });

  it('should compress Write tool', () => {
    const result = compressObservation('Write', {
      file_path: '/Users/dev/project/src/utils.ts',
      content: 'export function helper() {}',
    });
    expect(result).toBe('Created src/utils.ts (27 chars)');
  });

  it('should compress Bash tool', () => {
    const result = compressObservation(
      'Bash',
      { command: 'npm run build', description: 'Build the project' },
      'Build successful'
    );
    expect(result).toBe('Ran: npm run build - Build the project');
  });

  it('should compress unknown tools', () => {
    const result = compressObservation('CustomTool', {});
    expect(result).toBe('Used CustomTool');
  });

  it('should truncate long strings', () => {
    const longQuery = 'a'.repeat(100);
    const result = compressObservation('web_search', { query: longQuery });
    expect(result.length).toBeLessThan(120);
    expect(result).toContain('...');
  });
});

describe('compressMessagesForSummary', () => {
  const makeMessages = (): Message[] => [
    { role: 'user', content: [{ type: 'text', text: 'Search for my preferences' }] },
    {
      role: 'assistant',
      content: [
        { type: 'text', text: 'Let me search.' },
        { type: 'tool_use', id: 'tu_1', name: 'memory_search', input: { query: 'preferences' } },
      ],
    },
    {
      role: 'tool',
      content: [
        {
          type: 'tool_result',
          toolUseId: 'tu_1',
          content: 'Found 3 results:\n### Result 1...\n(500 chars of verbose output)',
        },
      ],
    },
    {
      role: 'assistant',
      content: [
        { type: 'text', text: 'I found your preferences.' },
        {
          type: 'tool_use',
          id: 'tu_2',
          name: 'memory_write',
          input: { path: 'memory.md', content: 'User likes dark mode' },
        },
      ],
    },
    {
      role: 'tool',
      content: [
        { type: 'tool_result', toolUseId: 'tu_2', content: 'Successfully wrote to memory.md' },
      ],
    },
  ];

  it('should skip memory_search tool_use and tool_result', () => {
    const messages = makeMessages();
    const compressed = compressMessagesForSummary(messages);

    // The assistant message that had memory_search tool_use should have it removed
    const assistantMsg = compressed.find(
      m =>
        m.role === 'assistant' &&
        m.content.some(c => c.type === 'text' && c.text === 'Let me search.')
    );
    expect(assistantMsg).toBeDefined();
    const hasSearchToolUse = assistantMsg!.content.some(
      c => c.type === 'tool_use' && c.name === 'memory_search'
    );
    expect(hasSearchToolUse).toBe(false);
  });

  it('should skip tool_result for skipped tools', () => {
    const messages = makeMessages();
    const compressed = compressMessagesForSummary(messages);

    // The tool message with memory_search result should be dropped entirely
    const searchResult = compressed.find(m =>
      m.content.some(c => c.type === 'tool_result' && c.toolUseId === 'tu_1')
    );
    expect(searchResult).toBeUndefined();
  });

  it('should compress captured tool results', () => {
    const messages = makeMessages();
    const compressed = compressMessagesForSummary(messages);

    // The memory_write tool_result should be compressed
    const writeResult = compressed.find(m =>
      m.content.some(c => c.type === 'tool_result' && c.toolUseId === 'tu_2')
    );
    expect(writeResult).toBeDefined();
    const resultBlock = writeResult!.content.find(
      c => c.type === 'tool_result' && c.toolUseId === 'tu_2'
    );
    expect(resultBlock).toBeDefined();
    if (resultBlock?.type === 'tool_result') {
      expect(resultBlock.content).toContain('Appended to memory.md');
    }
  });

  it('should preserve text blocks unchanged', () => {
    const messages = makeMessages();
    const compressed = compressMessagesForSummary(messages);

    const userMsg = compressed.find(m => m.role === 'user');
    expect(userMsg).toBeDefined();
    expect(userMsg!.content[0]).toEqual({ type: 'text', text: 'Search for my preferences' });
  });

  it('should respect custom config', () => {
    const messages = makeMessages();
    const customConfig: ToolCaptureConfig = {
      skipTools: ['memory_write'], // Skip write instead of search
      captureTools: ['memory_search'],
    };
    const compressed = compressMessagesForSummary(messages, customConfig);

    // memory_write tool_use should be dropped
    const hasWriteToolUse = compressed.some(m =>
      m.content.some(c => c.type === 'tool_use' && c.name === 'memory_write')
    );
    expect(hasWriteToolUse).toBe(false);
  });

  it('should drop messages with no remaining content', () => {
    const messages: Message[] = [
      {
        role: 'tool',
        content: [{ type: 'tool_result', toolUseId: 'tu_skip', content: 'skipped result' }],
      },
    ];

    // Map tool_use for the skip
    const fullMessages: Message[] = [
      {
        role: 'assistant',
        content: [
          { type: 'tool_use', id: 'tu_skip', name: 'memory_read', input: { path: 'test.md' } },
        ],
      },
      ...messages,
    ];

    const compressed = compressMessagesForSummary(fullMessages);
    // Both messages should be dropped (memory_read is in skipTools)
    expect(compressed.length).toBe(0);
  });
});

describe('DEFAULT_TOOL_CAPTURE_CONFIG', () => {
  it('should skip memory_read and memory_search', () => {
    expect(DEFAULT_TOOL_CAPTURE_CONFIG.skipTools).toContain('memory_read');
    expect(DEFAULT_TOOL_CAPTURE_CONFIG.skipTools).toContain('memory_search');
  });

  it('should capture memory_write and web_search', () => {
    expect(DEFAULT_TOOL_CAPTURE_CONFIG.captureTools).toContain('memory_write');
    expect(DEFAULT_TOOL_CAPTURE_CONFIG.captureTools).toContain('web_search');
  });
});
