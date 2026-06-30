import { describe, expect, it } from 'vitest';

import type { ToolTimelineEntry } from '../../store/chatRuntimeSlice';
import type { PersistedTranscriptItem } from '../../types/turnState';
import {
  buildProcessingBlocks,
  categorizeTool,
  formatTimelineEntry,
  formatToolName,
  isKnownClientTool,
  stripToolCallEnvelopes,
  summarizeToolGroup,
} from '../toolTimelineFormatting';

function entry(overrides: Partial<ToolTimelineEntry>): ToolTimelineEntry {
  return { id: 'x', name: 'delegate_notion', round: 1, status: 'running', ...overrides };
}

describe('formatTimelineEntry', () => {
  it('formats integration delegation tools with a user-facing provider label', () => {
    expect(
      formatTimelineEntry(
        entry({
          name: 'delegate_notion',
          argsBuffer: JSON.stringify({ prompt: 'Find the project brief in Notion.' }),
        })
      )
    ).toEqual({
      title: 'Working in your Notion workspace',
      detail: 'Find the project brief in Notion.',
    });
  });

  it('formats spawn_subagent for integrations_agent from toolkit args', () => {
    expect(
      formatTimelineEntry(
        entry({
          name: 'spawn_subagent',
          argsBuffer: JSON.stringify({
            agent_id: 'integrations_agent',
            prompt:
              'Get my 5 most recent emails. Show subject, sender, date, and a short preview for each.',
            toolkit: 'gmail',
          }),
        })
      )
    ).toEqual({
      title: 'Making requests to your Gmail account',
      detail:
        'Get my 5 most recent emails. Show subject, sender, date, and a short preview for each.',
    });
  });

  it('formats spawned integration agents with the inherited prompt', () => {
    expect(
      formatTimelineEntry(
        entry({
          name: 'subagent:integrations_agent',
          sourceToolName: 'delegate_notion',
          detail: 'Search Notion for the latest roadmap.',
        })
      )
    ).toEqual({
      title: 'Working in your Notion workspace',
      detail: 'Search Notion for the latest roadmap.',
    });
  });

  it('formats delegate_to_integrations_agent with a known toolkit arg', () => {
    expect(
      formatTimelineEntry(
        entry({
          name: 'delegate_to_integrations_agent',
          argsBuffer: JSON.stringify({
            toolkit: 'gmail',
            prompt: 'Find the latest invoice from Stripe.',
          }),
        })
      )
    ).toEqual({
      title: 'Making requests to your Gmail account',
      detail: 'Find the latest invoice from Stripe.',
    });
  });

  it('formats delegate_to_integrations_agent with an unknown toolkit arg', () => {
    expect(
      formatTimelineEntry(
        entry({
          name: 'delegate_to_integrations_agent',
          argsBuffer: JSON.stringify({ toolkit: 'slack_bot', prompt: 'post update' }),
        })
      )
    ).toEqual({ title: 'Checking your Slack Bot', detail: 'post update' });
  });

  it('formats delegate_to_integrations_agent without a toolkit arg as a generic connected-app label', () => {
    expect(
      formatTimelineEntry(
        entry({
          name: 'delegate_to_integrations_agent',
          argsBuffer: JSON.stringify({ prompt: 'do something useful' }),
        })
      )
    ).toEqual({ title: 'Checking your connected app', detail: 'do something useful' });
  });

  it('formats delegate_tools_agent with toolkit context from args', () => {
    expect(
      formatTimelineEntry(
        entry({
          name: 'delegate_tools_agent',
          argsBuffer: JSON.stringify({
            toolkit: 'github',
            prompt: 'List my open pull requests in GitHub.',
          }),
        })
      )
    ).toEqual({
      title: 'Making requests to your GitHub account',
      detail: 'List my open pull requests in GitHub.',
    });
  });

  it('falls back to humanized generic labels for non-integration subagents', () => {
    expect(formatTimelineEntry(entry({ name: 'subagent:researcher' }))).toEqual({
      title: 'Researching',
      detail: undefined,
    });
  });

  it('formats composio_list_connections with user-facing copy', () => {
    expect(formatTimelineEntry(entry({ name: 'composio_list_connections' }))).toEqual({
      title: 'Viewing your Connections',
      detail: undefined,
    });
  });

  it('formats shell tool with truncated command detail', () => {
    expect(
      formatTimelineEntry(
        entry({ name: 'shell', argsBuffer: JSON.stringify({ command: 'cargo test --lib' }) })
      )
    ).toEqual({ title: 'Running command', detail: 'cargo test --lib' });
  });

  it('formats web_fetch with hostname in title', () => {
    expect(
      formatTimelineEntry(
        entry({
          name: 'web_fetch',
          argsBuffer: JSON.stringify({ url: 'https://docs.example.com/api/v2/users' }),
        })
      )
    ).toEqual({
      title: 'Fetching docs.example.com',
      detail: 'https://docs.example.com/api/v2/users',
    });
  });

  it('formats web_search with query in title', () => {
    expect(
      formatTimelineEntry(
        entry({ name: 'web_search', argsBuffer: JSON.stringify({ query: 'rust async trait' }) })
      )
    ).toEqual({ title: 'Searching: rust async trait' });
  });

  it('formats file_read with shortened path', () => {
    expect(
      formatTimelineEntry(
        entry({
          name: 'file_read',
          argsBuffer: JSON.stringify({ path: 'src/openhuman/agent/progress.rs' }),
        })
      )
    ).toEqual({ title: 'Reading file', detail: '…/agent/progress.rs' });
  });

  it('formats edit tool with file path', () => {
    expect(
      formatTimelineEntry(
        entry({
          name: 'edit',
          argsBuffer: JSON.stringify({ file_path: 'app/src/components/App.tsx' }),
        })
      )
    ).toEqual({ title: 'Editing file', detail: '…/components/App.tsx' });
  });

  it('formats grep with pattern in title', () => {
    expect(
      formatTimelineEntry(
        entry({ name: 'grep', argsBuffer: JSON.stringify({ pattern: 'SubagentSpawned' }) })
      )
    ).toEqual({ title: 'Searching: SubagentSpawned' });
  });

  it('formats git_operations with subcommand', () => {
    expect(
      formatTimelineEntry(
        entry({ name: 'git_operations', argsBuffer: JSON.stringify({ command: 'diff --stat' }) })
      )
    ).toEqual({ title: 'Git diff', detail: 'diff --stat' });
  });

  it('formats screenshot as a simple label', () => {
    expect(formatTimelineEntry(entry({ name: 'screenshot' }))).toEqual({
      title: 'Taking screenshot',
    });
  });

  it('formats glob with pattern detail', () => {
    expect(
      formatTimelineEntry(
        entry({ name: 'glob', argsBuffer: JSON.stringify({ pattern: '**/*.test.ts' }) })
      )
    ).toEqual({ title: 'Finding: **/*.test.ts' });
  });

  it('formats list with directory path', () => {
    expect(
      formatTimelineEntry(
        entry({ name: 'list', argsBuffer: JSON.stringify({ path: 'src/openhuman/tools' }) })
      )
    ).toEqual({ title: 'Listing directory', detail: 'src/openhuman/tools' });
  });

  it('formats browser_open with hostname', () => {
    expect(
      formatTimelineEntry(
        entry({
          name: 'browser_open',
          argsBuffer: JSON.stringify({ url: 'https://github.com/tinyhumansai/openhuman' }),
        })
      )
    ).toEqual({ title: 'Browsing github.com' });
  });
});

describe('formatToolName', () => {
  it('returns human-readable names for known tools', () => {
    expect(formatToolName('shell')).toBe('Running command');
    expect(formatToolName('web_fetch')).toBe('Fetching');
    expect(formatToolName('file_read')).toBe('Reading file');
    expect(formatToolName('edit')).toBe('Editing file');
    expect(formatToolName('grep')).toBe('Searching code');
    expect(formatToolName('git_operations')).toBe('Git operation');
    expect(formatToolName('screenshot')).toBe('Taking screenshot');
    expect(formatToolName('lsp')).toBe('Code intelligence');
  });

  it('falls back to humanized identifier for unknown tools', () => {
    expect(formatToolName('custom_fancy_tool')).toBe('Custom Fancy Tool');
  });
});

describe('stripToolCallEnvelopes', () => {
  it('removes a complete <tool_call>…</tool_call> envelope, keeping the prose', () => {
    const out = stripToolCallEnvelopes(
      'Searching now. <tool_call> {"name": "NOTION_SEARCH", "arguments": {"q": "x"}} </tool_call> done.'
    );
    expect(out).toContain('Searching now.');
    expect(out).toContain('done.');
    expect(out).not.toContain('tool_call');
    expect(out).not.toContain('NOTION_SEARCH');
  });

  it('removes a trailing, still-streaming unclosed <tool_call>…', () => {
    const out = stripToolCallEnvelopes('Let me check. <tool_call> {"name": "X", "argum');
    expect(out.trim()).toBe('Let me check.');
    expect(out).not.toContain('tool_call');
  });

  it('leaves text without an envelope untouched', () => {
    expect(stripToolCallEnvelopes('just a normal sentence')).toBe('just a normal sentence');
  });
});

describe('isKnownClientTool', () => {
  it('recognizes built-ins and special agent rows', () => {
    expect(isKnownClientTool('file_read')).toBe(true);
    expect(isKnownClientTool('shell')).toBe(true);
    expect(isKnownClientTool('subagent:researcher')).toBe(true);
    expect(isKnownClientTool('delegate_to_integrations_agent')).toBe(true);
  });

  it('does not recognize dynamic Composio/MCP actions (server labels them)', () => {
    expect(isKnownClientTool('GMAIL_SEND_EMAIL')).toBe(false);
    expect(isKnownClientTool('composio_notion_create_page')).toBe(false);
    expect(isKnownClientTool('some_random_mcp_tool')).toBe(false);
  });
});

describe('summarizeToolGroup', () => {
  it('uses the specific label for a single row', () => {
    expect(summarizeToolGroup([entry({ name: 'file_read', argsBuffer: '{"path":"a.ts"}' })])).toBe(
      'Reading file'
    );
  });

  it('counts a homogeneous group', () => {
    expect(
      summarizeToolGroup([
        entry({ id: 'a', name: 'file_read' }),
        entry({ id: 'b', name: 'file_read' }),
      ])
    ).toBe('Read 2 files');
  });

  it('joins distinct category phrases for a mixed group', () => {
    expect(
      summarizeToolGroup([
        entry({ id: 'a', name: 'file_write' }),
        entry({ id: 'b', name: 'shell' }),
        entry({ id: 'c', name: 'shell' }),
      ])
    ).toBe('Edited 1 file, ran 2 commands');
  });
});

describe('categorizeTool', () => {
  it('maps tools (incl. subagent-prefixed) to a category', () => {
    expect(categorizeTool('grep')).toBe('search');
    expect(categorizeTool('subagent:web_fetch')).toBe('fetch');
    expect(categorizeTool('GMAIL_SEND_EMAIL')).toBe('other');
  });
});

describe('buildProcessingBlocks', () => {
  const tx = (over: Partial<PersistedTranscriptItem> & { kind: PersistedTranscriptItem['kind'] }) =>
    ({ round: 1, seq: 0, ...over }) as PersistedTranscriptItem;

  it('interleaves prose with grouped consecutive tool rows in seq order', () => {
    const entries = [
      entry({ id: 'c1', name: 'file_read' }),
      entry({ id: 'c2', name: 'file_read' }),
    ];
    const transcript: PersistedTranscriptItem[] = [
      tx({ kind: 'thinking', seq: 0, text: 'Let me look' }),
      tx({ kind: 'narration', seq: 1, text: 'Reading the files' }),
      tx({ kind: 'toolCall', seq: 2, callId: 'c1' }),
      tx({ kind: 'toolCall', seq: 3, callId: 'c2' }),
      tx({ kind: 'narration', seq: 4, text: 'Done' }),
    ];
    const blocks = buildProcessingBlocks(transcript, entries);
    expect(blocks.map(b => b.kind)).toEqual(['thinking', 'narration', 'toolGroup', 'narration']);
    const group = blocks[2];
    if (group.kind !== 'toolGroup') throw new Error('expected toolGroup');
    expect(group.summary).toBe('Read 2 files');
    expect(group.entries).toHaveLength(2);
  });

  it('skips tool pointers with no matching entry', () => {
    const blocks = buildProcessingBlocks([tx({ kind: 'toolCall', seq: 0, callId: 'missing' })], []);
    expect(blocks).toHaveLength(0);
  });

  it('falls back to a single group when there is no transcript', () => {
    const entries = [entry({ id: 'c1', name: 'shell' })];
    const blocks = buildProcessingBlocks([], entries);
    expect(blocks).toHaveLength(1);
    expect(blocks[0].kind).toBe('toolGroup');
  });
});

describe('formatter null-safety (malformed / legacy snapshot guard)', () => {
  it('does not throw on undefined input', () => {
    // A snake_case (legacy) persisted transcript item yields undefined
    // camelCase fields; the formatter must degrade, not crash the app.
    expect(formatToolName(undefined)).toBe('');
    expect(stripToolCallEnvelopes(undefined)).toBe('');
    expect(stripToolCallEnvelopes(null)).toBe('');
  });
});
