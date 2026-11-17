import { describe, expect, it } from 'vitest';

import type { DerivedDisplayItem } from '../../../types/derivedTranscript';
import { formatTimelineEntry } from '../../../utils/toolTimelineFormatting';
import { mapDisplayItems } from './mapDisplayItems';

/**
 * Build a newest-first page (as the RPC returns) from chronological items — the
 * mapper is responsible for reversing back to display order.
 */
function newestFirst(chronological: DerivedDisplayItem[]): DerivedDisplayItem[] {
  return [...chronological].reverse();
}

describe('mapDisplayItems', () => {
  /**
   * `callId` is whatever the provider wrote into the session transcript, and a
   * provider that emits tool calls without ids writes `''` for every one. Two
   * such calls in a turn used to collapse onto one row id, which lost the first
   * call from the timeline and gave assistant-ui two parts keyed
   * `toolCallId-` — a throw ("Duplicate key … in useResources") that takes the
   * thread render down on load.
   */
  it('gives id-less tool calls distinct, position-stable row ids', () => {
    const chronological: DerivedDisplayItem[] = [
      { kind: 'turnBoundary', requestId: 'req-1' },
      { kind: 'userMessage', content: 'go', requestId: 'req-1' },
      { kind: 'toolCall', callId: '', name: 'shell', status: 'success', result: 'one' },
      { kind: 'toolCall', callId: '', name: 'shell', status: 'success', result: 'two' },
    ];

    const { timelines, transcripts } = mapDisplayItems(newestFirst(chronological));

    const ids = timelines['req-1'].map(entry => entry.id);
    expect(ids).toHaveLength(2);
    expect(new Set(ids).size).toBe(2);
    expect(ids).not.toContain('');
    // Each transcript pointer names its own row, so both results survive.
    expect(transcripts['req-1'].map(item => 'callId' in item && item.callId)).toEqual(ids);
    expect(timelines['req-1'].map(entry => entry.result)).toEqual(['one', 'two']);

    // Position-derived, so re-deriving the same page yields the same ids — the
    // id has to survive a remount, not merely be unique once.
    expect(mapDisplayItems(newestFirst(chronological)).timelines['req-1'].map(e => e.id)).toEqual(
      ids
    );
  });

  it('disambiguates a repeated non-empty call id within a turn', () => {
    const chronological: DerivedDisplayItem[] = [
      { kind: 'turnBoundary', requestId: 'req-1' },
      { kind: 'userMessage', content: 'go', requestId: 'req-1' },
      { kind: 'toolCall', callId: 'dup', name: 'shell', status: 'success', result: 'one' },
      { kind: 'toolCall', callId: 'dup', name: 'shell', status: 'success', result: 'two' },
    ];

    const ids = mapDisplayItems(newestFirst(chronological)).timelines['req-1'].map(e => e.id);

    // The first keeps the provider's id; only the collision is renamed.
    expect(ids[0]).toBe('dup');
    expect(new Set(ids).size).toBe(2);
  });

  it('projects reasoning + interim narration + tool call for one turn, skipping the final answer', () => {
    const chronological: DerivedDisplayItem[] = [
      { kind: 'turnBoundary', requestId: 'req-1' },
      { kind: 'userMessage', content: 'hello', requestId: 'req-1' },
      { kind: 'reasoning', text: 'let me think' },
      { kind: 'assistantMessage', content: 'looking it up', interim: true, requestId: 'req-1' },
      {
        kind: 'toolCall',
        callId: 'call-a',
        name: 'shell',
        args: { cmd: 'ls' },
        result: 'file.txt',
        status: 'success',
      },
      { kind: 'assistantMessage', content: 'here is the answer', requestId: 'req-1' },
    ];

    const { timelines, transcripts, interrupted } = mapDisplayItems(newestFirst(chronological));

    // The final (non-interim) answer and the user text are NOT emitted — they
    // render from the thread message list.
    expect(interrupted).toEqual([]);
    expect(Object.keys(transcripts)).toEqual(['req-1']);
    expect(transcripts['req-1']).toEqual([
      { kind: 'thinking', round: 0, seq: 0, text: 'let me think' },
      { kind: 'narration', round: 0, seq: 1, text: 'looking it up' },
      { kind: 'toolCall', round: 0, seq: 2, callId: 'call-a' },
    ]);
    expect(timelines['req-1']).toEqual([
      expect.objectContaining({
        id: 'call-a',
        name: 'shell',
        seq: 2,
        status: 'success',
        argsBuffer: JSON.stringify({ cmd: 'ls' }),
        result: 'file.txt',
      }),
    ]);
  });

  it('preserves chronological (issue) order when reversing a newest-first page across turns', () => {
    const chronological: DerivedDisplayItem[] = [
      { kind: 'turnBoundary', requestId: 'req-1' },
      { kind: 'reasoning', text: 'turn one thought' },
      { kind: 'turnBoundary', requestId: 'req-2' },
      { kind: 'reasoning', text: 'turn two thought' },
    ];

    const { transcripts } = mapDisplayItems(newestFirst(chronological));

    expect(Object.keys(transcripts).sort()).toEqual(['req-1', 'req-2']);
    expect(transcripts['req-1']).toEqual([
      { kind: 'thinking', round: 0, seq: 0, text: 'turn one thought' },
    ]);
    expect(transcripts['req-2']).toEqual([
      { kind: 'thinking', round: 0, seq: 0, text: 'turn two thought' },
    ]);
  });

  it('maps a running (unpaired) tool call to a settled cancelled row', () => {
    const chronological: DerivedDisplayItem[] = [
      { kind: 'turnBoundary', requestId: 'req-1' },
      { kind: 'toolCall', callId: 'call-x', name: 'shell', status: 'running' },
    ];

    const { timelines } = mapDisplayItems(newestFirst(chronological));

    expect(timelines['req-1'][0]).toEqual(
      expect.objectContaining({ id: 'call-x', status: 'cancelled' })
    );
  });

  it('maps an error tool call to an error row', () => {
    const chronological: DerivedDisplayItem[] = [
      { kind: 'turnBoundary', requestId: 'req-1' },
      { kind: 'toolCall', callId: 'call-e', name: 'shell', status: 'error', result: 'boom' },
    ];

    const { timelines } = mapDisplayItems(newestFirst(chronological));

    expect(timelines['req-1'][0]).toEqual(
      expect.objectContaining({ id: 'call-e', status: 'error', result: 'boom' })
    );
  });

  it('maps a failed tool call onto a ToolFailureExplanation for ToolFailureLines', () => {
    const chronological: DerivedDisplayItem[] = [
      { kind: 'turnBoundary', requestId: 'req-1' },
      {
        kind: 'toolCall',
        callId: 'call-e',
        name: 'shell',
        status: 'error',
        result: 'boom',
        failure: { detail: 'exit 1: command not found' },
      },
    ];

    const { timelines } = mapDisplayItems(newestFirst(chronological));
    const row = timelines['req-1'][0];

    expect(row.status).toBe('error');
    expect(row.failure).toBeDefined();
    // The wire detail becomes the `causePlain` the ToolFailureLines renderer
    // shows for an unrecognised failure class.
    expect(row.failure?.causePlain).toBe('exit 1: command not found');
    expect(typeof row.failure?.class).toBe('string');
    expect(typeof row.failure?.nextAction).toBe('string');
  });

  it('falls back to the tool result as failure cause when no detail was captured', () => {
    const chronological: DerivedDisplayItem[] = [
      { kind: 'turnBoundary', requestId: 'req-1' },
      {
        kind: 'toolCall',
        callId: 'call-e',
        name: 'shell',
        status: 'error',
        result: 'raw error text',
        failure: {},
      },
    ];

    const { timelines } = mapDisplayItems(newestFirst(chronological));

    expect(timelines['req-1'][0].failure?.causePlain).toBe('raw error text');
  });

  it('derives the detail for a tool row but leaves displayName to the server', () => {
    const chronological: DerivedDisplayItem[] = [
      { kind: 'turnBoundary', requestId: 'req-1' },
      {
        kind: 'toolCall',
        callId: 'c1',
        name: 'shell',
        args: { command: 'ls -la' },
        result: 'ok',
        status: 'success',
      },
    ];

    const { timelines } = mapDisplayItems(newestFirst(chronological));
    const row = timelines['req-1'][0];

    // A baked client title froze its tense ("Running command" on a finished
    // row); surfaces resolve the title at render time instead.
    expect(row.displayName).toBeUndefined();
    expect(row.detail).toBe('ls -la');
    expect(formatTimelineEntry(row).title).toBe('Ran command');
  });

  it('anchors a subagent to its own requestId, not the current turn cursor', () => {
    // The subagent item is appended after both turns (as the projection emits
    // it) but belongs to req-1 via its core-derived requestId.
    const chronological: DerivedDisplayItem[] = [
      { kind: 'turnBoundary', requestId: 'req-1' },
      { kind: 'reasoning', text: 'turn one' },
      { kind: 'turnBoundary', requestId: 'req-2' },
      { kind: 'reasoning', text: 'turn two' },
      {
        kind: 'subagent',
        id: 'coder',
        requestId: 'req-1',
        items: [{ kind: 'assistantMessage', content: 'sub done', iteration: 1 }],
      },
    ];

    const { timelines } = mapDisplayItems(newestFirst(chronological));

    expect(timelines['req-1']?.some(e => e.name === 'subagent:coder')).toBe(true);
    expect(timelines['req-2']?.some(e => e.name === 'subagent:coder')).toBeFalsy();
  });

  it('projects a subagent item into a timeline row carrying its activity + transcript', () => {
    const chronological: DerivedDisplayItem[] = [
      { kind: 'turnBoundary', requestId: 'req-1' },
      {
        kind: 'subagent',
        id: 'researcher',
        items: [
          { kind: 'reasoning', text: 'child thinking' },
          { kind: 'assistantMessage', content: 'child answer', iteration: 1 },
          {
            kind: 'toolCall',
            callId: 'child-call',
            name: 'web_search',
            args: { q: 'x' },
            result: 'hits',
            status: 'success',
          },
        ],
      },
    ];

    const { timelines } = mapDisplayItems(newestFirst(chronological));
    const row = timelines['req-1'][0];

    expect(row.name).toBe('subagent:researcher');
    expect(row.subagent).toBeDefined();
    expect(row.subagent?.agentId).toBe('researcher');
    expect(row.subagent?.toolCalls).toEqual([
      expect.objectContaining({ callId: 'child-call', toolName: 'web_search', status: 'success' }),
    ]);
    expect(row.subagent?.transcript).toEqual([
      { kind: 'thinking', text: 'child thinking' },
      { kind: 'text', iteration: 1, text: 'child answer' },
      expect.objectContaining({ kind: 'tool', callId: 'child-call', toolName: 'web_search' }),
    ]);
  });

  it('collects interrupted partials by requestId and never emits them as trail items', () => {
    const chronological: DerivedDisplayItem[] = [
      { kind: 'turnBoundary', requestId: 'req-1' },
      { kind: 'interruptedPartial', text: 'half an ans', thinking: 'mid thought' },
    ];

    const { transcripts, timelines, interrupted } = mapDisplayItems(newestFirst(chronological));

    expect(interrupted).toEqual([
      { requestId: 'req-1', content: 'half an ans', thinking: 'mid thought' },
    ]);
    expect(transcripts['req-1']).toBeUndefined();
    expect(timelines['req-1']).toBeUndefined();
  });

  it('drops compaction markers (no settled-turn renderer)', () => {
    const chronological: DerivedDisplayItem[] = [
      { kind: 'turnBoundary', requestId: 'req-1' },
      { kind: 'compaction', replacedCount: 3, keptCount: 1 },
      { kind: 'reasoning', text: 'after compaction' },
    ];

    const { transcripts } = mapDisplayItems(newestFirst(chronological));

    expect(transcripts['req-1']).toEqual([
      { kind: 'thinking', round: 0, seq: 0, text: 'after compaction' },
    ]);
  });

  it('does not emit the final assistant or user text as trail items (dedupe vs thread messages)', () => {
    const chronological: DerivedDisplayItem[] = [
      { kind: 'turnBoundary', requestId: 'req-1' },
      {
        kind: 'userMessage',
        content: 'a question',
        displayContent: 'a question',
        requestId: 'req-1',
      },
      { kind: 'assistantMessage', content: 'a final answer', requestId: 'req-1' },
    ];

    const { transcripts, timelines } = mapDisplayItems(newestFirst(chronological));

    expect(transcripts['req-1']).toBeUndefined();
    expect(timelines['req-1']).toBeUndefined();
  });

  it('omits skipped request ids entirely', () => {
    const chronological: DerivedDisplayItem[] = [
      { kind: 'turnBoundary', requestId: 'req-old' },
      { kind: 'reasoning', text: 'old thought' },
      { kind: 'turnBoundary', requestId: 'req-live' },
      { kind: 'reasoning', text: 'live thought' },
    ];

    const { transcripts } = mapDisplayItems(newestFirst(chronological), {
      skipRequestIds: new Set(['req-live']),
    });

    expect(Object.keys(transcripts)).toEqual(['req-old']);
    expect(transcripts['req-live']).toBeUndefined();
  });

  it('carries the assistant iteration onto the turn round for its items', () => {
    const chronological: DerivedDisplayItem[] = [
      { kind: 'turnBoundary', requestId: 'req-1' },
      {
        kind: 'assistantMessage',
        content: 'step',
        interim: true,
        iteration: 2,
        requestId: 'req-1',
      },
      { kind: 'toolCall', callId: 'c1', name: 'shell', status: 'success' },
    ];

    const { transcripts, timelines } = mapDisplayItems(newestFirst(chronological));

    expect(transcripts['req-1'][0]).toEqual(
      expect.objectContaining({ kind: 'narration', round: 2 })
    );
    expect(timelines['req-1'][0].round).toBe(2);
  });

  /**
   * The core projects a step's reasoning *before* its message. The round used
   * to be taken only from the following assistantMessage, so each step's
   * reasoning was filed under the previous step.
   */
  it('files reasoning under the step it precedes, from its own iteration', () => {
    const chronological: DerivedDisplayItem[] = [
      { kind: 'turnBoundary', requestId: 'req-1' },
      { kind: 'reasoning', text: 'think one', iteration: 1 },
      {
        kind: 'assistantMessage',
        content: 'Let me check.',
        interim: true,
        iteration: 1,
        requestId: 'req-1',
      },
      { kind: 'toolCall', callId: 'c1', name: 'shell', status: 'success', iteration: 1 },
      { kind: 'reasoning', text: 'think two', iteration: 2 },
      // Step 2 has no narration: only its tool call says which step it is.
      { kind: 'toolCall', callId: 'c2', name: 'shell', status: 'success', iteration: 2 },
      { kind: 'reasoning', text: 'think three', iteration: 3 },
      { kind: 'assistantMessage', content: 'Done.', iteration: 3, requestId: 'req-1' },
    ];

    const { transcripts, timelines } = mapDisplayItems(newestFirst(chronological));

    const thinking = transcripts['req-1']
      .filter(item => item.kind === 'thinking')
      .map(item => ('text' in item ? [item.text, item.round] : []));
    expect(thinking).toEqual([
      ['think one', 1],
      ['think two', 2],
      ['think three', 3],
    ]);
    expect(timelines['req-1'].map(entry => [entry.id, entry.round])).toEqual([
      ['c1', 1],
      ['c2', 2],
    ]);
  });

  it('keys sub-agent rows by their unique run id and maps their terminal status', () => {
    const chronological: DerivedDisplayItem[] = [
      { kind: 'turnBoundary', requestId: 'req-1' },
      { kind: 'toolCall', callId: 'c1', name: 'research', status: 'success', iteration: 1 },
      {
        kind: 'subagent',
        id: 'sub-aaa',
        agentId: 'researcher',
        taskId: 'sub-aaa',
        callId: 'c1',
        status: 'completed',
        requestId: 'req-1',
        items: [],
      },
      { kind: 'toolCall', callId: 'c2', name: 'research', status: 'error', iteration: 2 },
      {
        kind: 'subagent',
        id: 'sub-bbb',
        agentId: 'researcher',
        taskId: 'sub-bbb',
        callId: 'c2',
        status: 'failed',
        requestId: 'req-1',
        items: [],
      },
    ];

    const rows = mapDisplayItems(newestFirst(chronological)).timelines['req-1'];

    // Each run follows its spawning call, and two runs of one agent no longer
    // share `subagent:researcher` as their id.
    expect(rows.map(row => row.id)).toEqual(['c1', 'subagent:sub-aaa', 'c2', 'subagent:sub-bbb']);
    const [, first, , second] = rows;
    expect(first.name).toBe('subagent:researcher');
    expect(first.status).toBe('success');
    expect(first.subagent).toEqual(
      expect.objectContaining({ taskId: 'sub-aaa', agentId: 'researcher', status: 'completed' })
    );
    expect(second.status).toBe('error');
    expect(second.subagent?.status).toBe('failed');
  });

  it('settles an incomplete sub-agent as a non-success partial state', () => {
    const rows = mapDisplayItems(
      newestFirst([
        { kind: 'turnBoundary', requestId: 'req-i' },
        { kind: 'toolCall', callId: 'ci', name: 'research', status: 'success', iteration: 1 },
        {
          kind: 'subagent',
          id: 'sub-inc',
          agentId: 'researcher',
          taskId: 'sub-inc',
          callId: 'ci',
          status: 'incomplete',
          requestId: 'req-i',
          items: [],
        },
      ])
    ).timelines['req-i'];

    const sub = rows.find(row => row.id === 'subagent:sub-inc');
    expect(sub?.status).not.toBe('success');
    expect(sub?.status).toBe('cancelled');
    expect(sub?.subagent?.status).toBe('incomplete');
  });

  describe('sub-agent placement (live parity)', () => {
    /**
     * Live, `subagentSpawned` turns the `spawn_subagent` row into the
     * delegation card IN ITS SLOT. The projection nests sub-agents after every
     * root item, so a reopened turn used to show the spawn call as a plain
     * tool card and the delegation card at the bottom — a different turn from
     * the one the user watched stream.
     */
    it('puts the sub-agent in the slot of the call that spawned it', () => {
      const { timelines, transcripts } = mapDisplayItems(
        newestFirst([
          { kind: 'turnBoundary', requestId: 'req-1' },
          { kind: 'toolCall', callId: 'call-a', name: 'shell', status: 'success' },
          {
            kind: 'toolCall',
            callId: 'call-spawn',
            name: 'spawn_subagent',
            args: { prompt: 'dig in' },
            status: 'success',
          },
          { kind: 'toolCall', callId: 'call-b', name: 'shell', status: 'success' },
          { kind: 'subagent', id: 'researcher', requestId: 'req-1', items: [] },
        ])
      );
      const rows = timelines['req-1'] ?? [];
      expect(rows.map(row => row.id)).toEqual(['call-a', 'subagent:researcher', 'call-b']);
      expect(rows[1]?.seq).toBeLessThan(rows[2]?.seq ?? -1);
      expect(rows[1]?.sourceToolName).toBe('spawn_subagent');
      // The transcript pointer follows the row, so the card renders in place.
      const pointers = (transcripts['req-1'] ?? []).flatMap(item =>
        item.kind === 'toolCall' ? [item.callId] : []
      );
      expect(pointers).toEqual(['call-a', 'subagent:researcher', 'call-b']);
    });

    it('pairs several sub-agents with their spawn calls in order', () => {
      const { timelines } = mapDisplayItems(
        newestFirst([
          { kind: 'turnBoundary', requestId: 'req-1' },
          { kind: 'toolCall', callId: 's1', name: 'spawn_subagent', status: 'success' },
          { kind: 'toolCall', callId: 's2', name: 'delegate_coder', status: 'success' },
          { kind: 'subagent', id: 'researcher', requestId: 'req-1', items: [] },
          { kind: 'subagent', id: 'researcher', requestId: 'req-1', items: [] },
        ])
      );
      expect((timelines['req-1'] ?? []).map(row => row.id)).toEqual([
        'subagent:researcher',
        'subagent:researcher#2',
      ]);
    });

    it('appends a sub-agent with no spawn call to pair with, as before', () => {
      const { timelines } = mapDisplayItems(
        newestFirst([
          { kind: 'turnBoundary', requestId: 'req-1' },
          { kind: 'toolCall', callId: 'call-a', name: 'shell', status: 'success' },
          { kind: 'subagent', id: 'coder', requestId: 'req-1', items: [] },
        ])
      );
      expect((timelines['req-1'] ?? []).map(row => row.id)).toEqual(['call-a', 'subagent:coder']);
    });

    it('folds a delegation the core names by callId into that call’s slot', () => {
      const { timelines } = mapDisplayItems(
        newestFirst([
          { kind: 'turnBoundary', requestId: 'req-1' },
          { kind: 'toolCall', callId: 'call-spawn', name: 'spawn_subagent', status: 'success' },
          {
            kind: 'subagent',
            id: 'sub-1',
            agentId: 'researcher',
            callId: 'call-spawn',
            status: 'completed',
            requestId: 'req-1',
            items: [],
          },
          { kind: 'toolCall', callId: 'call-b', name: 'shell', status: 'success' },
        ])
      );
      expect((timelines['req-1'] ?? []).map(row => row.id)).toEqual(['subagent:sub-1', 'call-b']);
    });
  });
});
