import { describe, expect, it } from 'vitest';

import type { ToolTimelineEntry } from '../../store/chatRuntimeSlice';
import { formatTimelineEntry } from '../toolTimelineFormatting';

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
    ).toEqual({ title: 'Checking your Notion', detail: 'Find the project brief in Notion.' });
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
      title: 'Checking your Gmail',
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
    ).toEqual({ title: 'Checking your Notion', detail: 'Search Notion for the latest roadmap.' });
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
});
