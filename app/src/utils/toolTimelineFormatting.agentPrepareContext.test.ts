import { describe, expect, it } from 'vitest';

import type { ToolTimelineEntry } from '../store/chatRuntimeSlice';
import { formatTimelineEntry, formatToolName } from './toolTimelineFormatting';

function entry(partial: Partial<ToolTimelineEntry> & { name: string }): ToolTimelineEntry {
  return { id: 'e1', round: 0, status: 'running', ...partial };
}

describe('toolTimelineFormatting — agent_prepare_context / context_scout', () => {
  it('labels the agent_prepare_context tool name', () => {
    expect(formatToolName('agent_prepare_context')).toBe('Preparing context');
  });

  it('formats the agent_prepare_context tool entry with the question as detail', () => {
    const result = formatTimelineEntry(
      entry({
        name: 'agent_prepare_context',
        argsBuffer: JSON.stringify({ question: 'what should I focus on?' }),
      })
    );
    expect(result.title).toBe('Preparing context');
    expect(result.detail).toBe('what should I focus on?');
  });

  it('formats the context_scout subagent rows', () => {
    expect(formatTimelineEntry(entry({ name: 'subagent:context_scout', detail: 'd' })).title).toBe(
      'Scouting context'
    );
    expect(formatTimelineEntry(entry({ name: 'context_scout' })).title).toBe('Scouting context');
  });
});
