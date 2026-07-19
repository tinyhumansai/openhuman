import { describe, expect, it } from 'vitest';

import { mergeToolActivity, type ToolActivity, toolResultFailed } from './mergeToolActivity';
import type { ChatMessage } from './useOrchestrationChats';

const msg = (over: Partial<ChatMessage>): ChatMessage => ({
  id: 'x',
  from: 'agent',
  body: '',
  timestamp: '2026-07-08T00:00:00Z',
  encrypted: false,
  ...over,
});

describe('toolResultFailed', () => {
  it('flags isError, non-zero exit, or ok=false; passes success', () => {
    expect(toolResultFailed({ ok: true, isError: false, exitCode: 0 })).toBe(false);
    expect(toolResultFailed({ isError: true })).toBe(true);
    expect(toolResultFailed({ exitCode: 1 })).toBe(true);
    expect(toolResultFailed({ ok: false })).toBe(true);
    expect(toolResultFailed({})).toBe(false);
  });
});

describe('mergeToolActivity', () => {
  it('merges a tool_call with its tool_result by callId', () => {
    const rows = mergeToolActivity([
      msg({ id: 'a', eventKind: 'tool_call', toolName: 'Bash', callId: 'c1', body: 'ls' }),
      msg({ id: 'b', eventKind: 'tool_result', callId: 'c1', body: 'out', ok: true }),
    ]);
    expect(rows).toHaveLength(1);
    const tool = rows[0] as ToolActivity;
    expect(tool.kind).toBe('tool');
    expect(tool.command).toBe('ls');
    expect(tool.output).toBe('out');
    expect(tool.toolName).toBe('Bash');
    expect(tool.hasResult).toBe(true);
    expect(tool.failed).toBe(false);
  });

  it('marks a failed result (isError / exitCode)', () => {
    const rows = mergeToolActivity([
      msg({ id: 'a', eventKind: 'tool_call', callId: 'c1', body: 'read x' }),
      msg({
        id: 'b',
        eventKind: 'tool_result',
        callId: 'c1',
        body: 'boom',
        isError: true,
        exitCode: 1,
        ok: false,
      }),
    ]);
    expect((rows[0] as ToolActivity).failed).toBe(true);
  });

  it('keeps a tool_call open (no result) as an unfinished tool row', () => {
    const rows = mergeToolActivity([
      msg({ id: 'a', eventKind: 'tool_call', callId: 'c1', body: 'ls' }),
    ]);
    expect(rows).toHaveLength(1);
    expect((rows[0] as ToolActivity).hasResult).toBe(false);
  });

  it('renders an orphan tool_result (no prior call) standalone', () => {
    const rows = mergeToolActivity([
      msg({
        id: 'r',
        eventKind: 'tool_result',
        callId: 'zz',
        body: 'late',
        ok: true,
        toolName: 'Read',
      }),
    ]);
    expect(rows).toHaveLength(1);
    const tool = rows[0] as ToolActivity;
    expect(tool.command).toBe('');
    expect(tool.output).toBe('late');
    expect(tool.toolName).toBe('Read');
    expect(tool.hasResult).toBe(true);
  });

  it('passes non-tool rows through as message rows, preserving order', () => {
    const rows = mergeToolActivity([
      msg({ id: 'u', eventKind: 'user_prompt', body: 'hi' }),
      msg({ id: 'a', eventKind: 'tool_call', callId: 'c1', body: 'ls' }),
      msg({ id: 'b', eventKind: 'tool_result', callId: 'c1', body: 'out' }),
      msg({ id: 'm', eventKind: 'agent_message', body: 'done' }),
    ]);
    expect(rows.map(r => r.kind)).toEqual(['message', 'tool', 'message']);
    expect(rows[0]).toMatchObject({ kind: 'message', message: { id: 'u' } });
    expect(rows[2]).toMatchObject({ kind: 'message', message: { id: 'm' } });
  });

  it('handles interleaved concurrent tool calls', () => {
    const rows = mergeToolActivity([
      msg({ id: 'a', eventKind: 'tool_call', callId: 'c1', body: 'one' }),
      msg({ id: 'b', eventKind: 'tool_call', callId: 'c2', body: 'two' }),
      msg({ id: 'c', eventKind: 'tool_result', callId: 'c2', body: 'two-out' }),
      msg({ id: 'd', eventKind: 'tool_result', callId: 'c1', body: 'one-out' }),
    ]);
    expect(rows).toHaveLength(2);
    expect((rows[0] as ToolActivity).output).toBe('one-out');
    expect((rows[1] as ToolActivity).output).toBe('two-out');
  });
});
