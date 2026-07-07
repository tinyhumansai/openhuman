import { describe, expect, it } from 'vitest';

import type { StreamingAssistantState, ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import type { RootState } from '../../../store/index';
import type { ThreadMessage } from '../../../types/thread';
import { buildThreadTimeline, groupTimelineIntoTurns, selectTimelineForThread } from './selectors';
import { LEGACY_TURN_ID, type TimelineItem } from './types';

const THREAD = 't1';

function userMsg(
  id: string,
  content = `u-${id}`,
  extra: Record<string, unknown> = {}
): ThreadMessage {
  return {
    id,
    content,
    type: 'text',
    extraMetadata: extra,
    sender: 'user',
    createdAt: '2026-01-01T00:00:00Z',
  };
}
function agentMsg(
  id: string,
  content = `a-${id}`,
  extra: Record<string, unknown> = {}
): ThreadMessage {
  return {
    id,
    content,
    type: 'text',
    extraMetadata: extra,
    sender: 'agent',
    createdAt: '2026-01-01T00:00:01Z',
  };
}
function tool(
  id: string,
  name: string,
  round = 0,
  status: ToolTimelineEntry['status'] = 'success'
): ToolTimelineEntry {
  return { id, name, round, status };
}
function subagentRow(id: string, taskId: string, round = 0): ToolTimelineEntry {
  return {
    id,
    name: 'subagent:researcher',
    round,
    status: 'running',
    subagent: { taskId, agentId: 'researcher', toolCalls: [] },
  };
}
function stream(requestId: string, content: string, thinking = ''): StreamingAssistantState {
  return { requestId, content, thinking };
}

function build(partial: Partial<Parameters<typeof buildThreadTimeline>[0]>): TimelineItem[] {
  return buildThreadTimeline({
    threadId: THREAD,
    messages: [],
    toolTimeline: [],
    streaming: null,
    parallelStreams: [],
    hideAgentInsights: false,
    ...partial,
  });
}

const kinds = (items: TimelineItem[]) => items.map(i => i.kind);
const ids = (items: TimelineItem[]) => items.map(i => i.id);

describe('buildThreadTimeline — ordering', () => {
  it('emits messages in stored order with monotonic seq, all in the legacy turn', () => {
    const items = build({ messages: [userMsg('u1'), agentMsg('a1')] });
    expect(kinds(items)).toEqual(['userMessage', 'assistantMessage']);
    expect(items.map(i => i.seq)).toEqual([0, 1]);
    expect(items.every(i => i.turnId === LEGACY_TURN_ID)).toBe(true);
    expect(items.every(i => i.threadId === THREAD)).toBe(true);
  });

  it('anchors the tool timeline immediately after the last user message', () => {
    // Thread: u1, a1, u2, a2 — anchor is u2, so process rows sit between u2 and a2.
    const items = build({
      messages: [userMsg('u1'), agentMsg('a1'), userMsg('u2'), agentMsg('a2')],
      toolTimeline: [tool('call1', 'read_file', 0), tool('call2', 'write_file', 1)],
    });
    expect(kinds(items)).toEqual([
      'userMessage', // u1
      'assistantMessage', // a1
      'userMessage', // u2 (anchor)
      'toolCall', // call1
      'toolCall', // call2
      'assistantMessage', // a2
    ]);
    expect(ids(items)).toEqual(['u1', 'a1', 'u2', 'call1', 'call2', 'a2']);
    // seq is monotonic across the spliced process block.
    expect(items.map(i => i.seq)).toEqual([0, 1, 2, 3, 4, 5]);
  });

  it('preserves tool-timeline array order and maps success→ok', () => {
    const items = build({
      messages: [userMsg('u1')],
      toolTimeline: [
        tool('c1', 'a', 0, 'success'),
        tool('c2', 'b', 0, 'error'),
        tool('c3', 'c', 1, 'running'),
      ],
    });
    const calls = items.filter(i => i.kind === 'toolCall');
    expect(calls.map(c => (c.kind === 'toolCall' ? c.status : null))).toEqual([
      'ok',
      'error',
      'running',
    ]);
    expect(calls.map(c => c.id)).toEqual(['c1', 'c2', 'c3']);
  });

  it('classifies subagent:* rows as subagentActivity with the taskId', () => {
    const items = build({
      messages: [userMsg('u1')],
      toolTimeline: [subagentRow('row1', 'sub-42', 0)],
    });
    const sub = items.find(i => i.kind === 'subagentActivity');
    expect(sub).toBeDefined();
    expect(sub && sub.kind === 'subagentActivity' && sub.taskId).toBe('sub-42');
  });
});

describe('buildThreadTimeline — legacy / proactive fallbacks', () => {
  it('empty thread yields no items', () => {
    expect(build({})).toEqual([]);
  });

  it('proactive thread with no user message trails process items after the messages (L2669 fallback)', () => {
    const items = build({
      messages: [agentMsg('a1'), agentMsg('a2')],
      toolTimeline: [tool('c1', 'read_file', 0)],
    });
    expect(kinds(items)).toEqual(['assistantMessage', 'assistantMessage', 'toolCall']);
    expect(ids(items)).toEqual(['a1', 'a2', 'c1']);
  });

  it('drops hidden messages before anchoring', () => {
    const items = build({
      messages: [userMsg('u1'), userMsg('u2', 'hidden', { hidden: true }), agentMsg('a1')],
      toolTimeline: [tool('c1', 'x', 0)],
    });
    // u2 is hidden, so the anchor is u1 — process rows sit between u1 and a1.
    expect(ids(items)).toEqual(['u1', 'c1', 'a1']);
  });
});

describe('buildThreadTimeline — hideAgentInsights', () => {
  it('omits tool/subagent process items when hidden but keeps messages', () => {
    const items = build({
      messages: [userMsg('u1'), agentMsg('a1')],
      toolTimeline: [tool('c1', 'x', 0), subagentRow('row1', 'sub-1', 0)],
      hideAgentInsights: true,
    });
    expect(kinds(items)).toEqual(['userMessage', 'assistantMessage']);
  });

  it('keeps the streaming preview (answer) even when insights are hidden', () => {
    const items = build({
      messages: [userMsg('u1')],
      toolTimeline: [tool('c1', 'x', 0)],
      streaming: stream('req1', 'partial answer', 'private thoughts'),
      hideAgentInsights: true,
    });
    expect(kinds(items)).toEqual(['userMessage', 'streamingText']);
  });
});

describe('buildThreadTimeline — streaming previews', () => {
  it('trails the primary streaming item after durable items, carrying thinking', () => {
    const items = build({
      messages: [userMsg('u1'), agentMsg('a1')],
      streaming: stream('req1', 'hello', 'thinking...'),
    });
    const last = items[items.length - 1];
    expect(last.kind).toBe('streamingText');
    if (last.kind === 'streamingText') {
      expect(last.text).toBe('hello');
      expect(last.thinking).toBe('thinking...');
      expect(last.branch).toBe(false);
      expect(last.streamId).toBe('req1');
    }
  });

  it('skips an empty primary stream but renders forked branches (content only)', () => {
    const items = build({
      messages: [userMsg('u1')],
      streaming: stream('req1', '', ''),
      parallelStreams: [stream('b1', 'branch one'), stream('b2', '', '')],
    });
    const streams = items.filter(i => i.kind === 'streamingText');
    expect(streams).toHaveLength(1);
    const branch = streams[0];
    expect(branch.kind === 'streamingText' && branch.branch).toBe(true);
    expect(branch.kind === 'streamingText' && branch.text).toBe('branch one');
    // Branch thinking is not carried (rendered content-only today).
    expect(branch.kind === 'streamingText' && branch.thinking).toBeUndefined();
  });
});

describe('buildThreadTimeline — requestId grouping (Phase 4 anchoring)', () => {
  it('derives turnId from extraMetadata.requestId when present', () => {
    const items = build({
      messages: [
        userMsg('u1', 'q1', { requestId: 'req-1' }),
        agentMsg('a1', 'ans1', { requestId: 'req-1' }),
        userMsg('u2', 'q2', { requestId: 'req-2' }),
        agentMsg('a2', 'ans2', { requestId: 'req-2' }),
      ],
    });
    expect(items.map(i => i.turnId)).toEqual(['req-1', 'req-1', 'req-2', 'req-2']);
  });

  it('falls back to the legacy turn for messages without a requestId', () => {
    const items = build({
      messages: [userMsg('u1'), agentMsg('a1', 'ans', { requestId: 'req-9' })],
    });
    expect(items.map(i => i.turnId)).toEqual([LEGACY_TURN_ID, 'req-9']);
  });

  it('groups a mixed thread into per-request turns', () => {
    const items = build({
      messages: [
        userMsg('u1', 'q1', { requestId: 'req-1' }),
        agentMsg('a1', 'ans1', { requestId: 'req-1' }),
        userMsg('u2', 'q2', { requestId: 'req-2' }),
        agentMsg('a2', 'ans2', { requestId: 'req-2' }),
      ],
    });
    const turns = groupTimelineIntoTurns(items);
    expect(turns.map(t => t.turnId)).toEqual(['req-1', 'req-2']);
    expect(turns.map(t => t.items.length)).toEqual([2, 2]);
  });
});

describe('groupTimelineIntoTurns', () => {
  it('groups a single legacy turn into one group', () => {
    const items = build({
      messages: [userMsg('u1'), agentMsg('a1')],
      toolTimeline: [tool('c1', 'x')],
    });
    const turns = groupTimelineIntoTurns(items);
    expect(turns).toHaveLength(1);
    expect(turns[0].turnId).toBe(LEGACY_TURN_ID);
    expect(turns[0].items).toHaveLength(3);
  });

  it('empty input yields no turns', () => {
    expect(groupTimelineIntoTurns([])).toEqual([]);
  });
});

describe('selectTimelineForThread (memoized)', () => {
  function stateWith(
    over: Partial<RootState['chatRuntime']> = {},
    theme = { hideAgentInsights: false }
  ): RootState {
    return {
      thread: { messagesByThreadId: { [THREAD]: [userMsg('u1'), agentMsg('a1')] } },
      chatRuntime: {
        toolTimelineByThread: { [THREAD]: [tool('c1', 'read_file', 0)] },
        streamingAssistantByThread: {},
        parallelStreamsByThread: {},
        ...over,
      },
      theme,
    } as unknown as RootState;
  }

  it('projects from state and anchors the tool row after the user message', () => {
    const items = selectTimelineForThread(stateWith(), THREAD);
    expect(ids(items)).toEqual(['u1', 'c1', 'a1']);
  });

  it('returns a referentially stable result for unchanged inputs (memoization)', () => {
    const state = stateWith();
    const a = selectTimelineForThread(state, THREAD);
    const b = selectTimelineForThread(state, THREAD);
    expect(a).toBe(b);
  });

  it('honors hideAgentInsights from theme state', () => {
    const items = selectTimelineForThread(stateWith({}, { hideAgentInsights: true }), THREAD);
    expect(kinds(items)).toEqual(['userMessage', 'assistantMessage']);
  });

  it('returns an empty projection for an unknown thread', () => {
    expect(selectTimelineForThread(stateWith(), 'nope')).toEqual([]);
  });
});
