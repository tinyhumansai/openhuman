import { describe, expect, it } from 'vitest';

import reducer, {
  appendSubagentStreamDelta,
  recordSubagentTranscriptTool,
  resolveSubagentTranscriptTool,
  setToolTimelineForThread,
  type SubagentTranscriptItem,
  type ToolTimelineEntry,
} from '../chatRuntimeSlice';

const THREAD = 'thread-1';
const ROW_ID = `${THREAD}:subagent:sub-1:researcher`;

function withSubagentRow(): ReturnType<typeof reducer> {
  const entry: ToolTimelineEntry = {
    id: ROW_ID,
    name: 'subagent:researcher',
    round: 1,
    status: 'running',
    subagent: { taskId: 'sub-1', agentId: 'researcher', toolCalls: [], transcript: [] },
  };
  return reducer(undefined, setToolTimelineForThread({ threadId: THREAD, entries: [entry] }));
}

function transcriptOf(state: ReturnType<typeof reducer>): SubagentTranscriptItem[] {
  return state.toolTimelineByThread[THREAD][0].subagent?.transcript ?? [];
}

describe('subagent transcript reducers', () => {
  it('accumulates consecutive same-kind deltas into one transcript item', () => {
    let state = withSubagentRow();
    state = reducer(
      state,
      appendSubagentStreamDelta({
        threadId: THREAD,
        rowId: ROW_ID,
        kind: 'text',
        delta: 'Hello ',
        iteration: 1,
      })
    );
    state = reducer(
      state,
      appendSubagentStreamDelta({
        threadId: THREAD,
        rowId: ROW_ID,
        kind: 'text',
        delta: 'world',
        iteration: 1,
      })
    );
    const t = transcriptOf(state);
    expect(t).toHaveLength(1);
    expect(t[0]).toMatchObject({ kind: 'text', text: 'Hello world', iteration: 1 });
  });

  it('does not merge same-kind deltas from different iterations', () => {
    let state = withSubagentRow();
    state = reducer(
      state,
      appendSubagentStreamDelta({
        threadId: THREAD,
        rowId: ROW_ID,
        kind: 'text',
        delta: 'turn one',
        iteration: 1,
      })
    );
    state = reducer(
      state,
      appendSubagentStreamDelta({
        threadId: THREAD,
        rowId: ROW_ID,
        kind: 'text',
        delta: 'turn two',
        iteration: 2,
      })
    );
    const t = transcriptOf(state);
    expect(t).toHaveLength(2);
    expect(t.map(i => (i.kind === 'text' ? i.iteration : null))).toEqual([1, 2]);
  });

  it('interleaves thinking, text, and tool calls in arrival order', () => {
    let state = withSubagentRow();
    // turn 1: think → text → tool call
    state = reducer(
      state,
      appendSubagentStreamDelta({
        threadId: THREAD,
        rowId: ROW_ID,
        kind: 'thinking',
        delta: 'I should search',
        iteration: 1,
      })
    );
    state = reducer(
      state,
      appendSubagentStreamDelta({
        threadId: THREAD,
        rowId: ROW_ID,
        kind: 'text',
        delta: 'Let me look that up.',
        iteration: 1,
      })
    );
    state = reducer(
      state,
      recordSubagentTranscriptTool({
        threadId: THREAD,
        rowId: ROW_ID,
        callId: 'c1',
        toolName: 'web_search',
        iteration: 1,
      })
    );
    // turn 2: text only (final answer)
    state = reducer(
      state,
      appendSubagentStreamDelta({
        threadId: THREAD,
        rowId: ROW_ID,
        kind: 'text',
        delta: 'Found it.',
        iteration: 2,
      })
    );

    const t = transcriptOf(state);
    expect(t.map(i => i.kind)).toEqual(['thinking', 'text', 'tool', 'text']);
    // The tool sits between the turn-1 text and the turn-2 text — i.e. where
    // it actually occurred, not bucketed into a separate section.
    expect(t[2]).toMatchObject({ kind: 'tool', callId: 'c1', status: 'running' });
    expect(t[3]).toMatchObject({ kind: 'text', text: 'Found it.', iteration: 2 });
  });

  it('resolves a transcript tool item to its terminal status with timing', () => {
    let state = withSubagentRow();
    state = reducer(
      state,
      recordSubagentTranscriptTool({
        threadId: THREAD,
        rowId: ROW_ID,
        callId: 'c1',
        toolName: 'web_search',
        iteration: 1,
      })
    );
    state = reducer(
      state,
      resolveSubagentTranscriptTool({
        threadId: THREAD,
        rowId: ROW_ID,
        callId: 'c1',
        success: true,
        elapsedMs: 1200,
        outputChars: 480,
      })
    );
    const tool = transcriptOf(state).find(i => i.kind === 'tool');
    expect(tool).toMatchObject({ status: 'success', elapsedMs: 1200, outputChars: 480 });
  });

  it('de-dupes a redelivered tool-call event by callId', () => {
    let state = withSubagentRow();
    const record = recordSubagentTranscriptTool({
      threadId: THREAD,
      rowId: ROW_ID,
      callId: 'c1',
      toolName: 'web_search',
    });
    state = reducer(state, record);
    state = reducer(state, record);
    expect(transcriptOf(state).filter(i => i.kind === 'tool')).toHaveLength(1);
  });

  it('is a no-op when the thread or row is unknown', () => {
    const state = withSubagentRow();
    expect(
      reducer(
        state,
        appendSubagentStreamDelta({ threadId: 'nope', rowId: ROW_ID, kind: 'text', delta: 'x' })
      )
    ).toEqual(state);
    const unknownRow = reducer(
      state,
      appendSubagentStreamDelta({ threadId: THREAD, rowId: 'missing', kind: 'text', delta: 'x' })
    );
    expect(transcriptOf(unknownRow)).toHaveLength(0);
  });
});
