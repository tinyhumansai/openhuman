import { describe, expect, it } from 'vitest';

import type { DerivedDisplayItem } from '../../../types/derivedTranscript';
import { mapDisplayItems } from './mapDisplayItems';

/**
 * Build a newest-first page (as the RPC returns) from chronological items — the
 * mapper is responsible for reversing back to display order.
 */
function newestFirst(chronological: DerivedDisplayItem[]): DerivedDisplayItem[] {
  return [...chronological].reverse();
}

describe('mapDisplayItems', () => {
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

  it('derives displayName/detail for a tool row (parity with turn_state rows)', () => {
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

    expect(typeof row.displayName).toBe('string');
    expect(row.displayName?.length ?? 0).toBeGreaterThan(0);
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
});
