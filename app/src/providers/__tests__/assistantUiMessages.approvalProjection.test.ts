/**
 * Where a parked approval lands in the projection, and what must not happen
 * while it is parked.
 *
 * `AssistantUiRuntimeProvider.approvalGate.test.tsx` covers the happy path
 * end-to-end: a gate parks mid-turn, the prompt appears on the call, a decision
 * routes to `openhuman.approval_decide`. All four of its cases run with the
 * turn *live* (`beginInferenceTurn` + `markInferenceTurnStreaming`) and with a
 * single matching timeline row, so three rules `buildRuntimeMessages` encodes
 * for the parked state have no test that fails when they are removed:
 *
 *  1. the gate attaches to a call that has NOT settled, even when an earlier
 *     call of the same name already ran;
 *  2. the tail is minted even when the projection reports the turn finished;
 *  3. and in that case the live rows are emitted once, not twice.
 *
 * (3) is the one with teeth. assistant-ui throws on a repeated `toolCallId`
 * rather than dropping the duplicate, so a regression there is not a cosmetic
 * double-render — it takes the whole transcript down at the exact moment the
 * user is being asked to unblock the turn.
 */
import { describe, expect, it } from 'vitest';

import type { PendingApproval, ToolTimelineEntry } from '../../store/chatRuntimeSlice';
import type { ThreadMessage } from '../../types/thread';
import { buildRuntimeMessages, STREAMING_TAIL_ID } from '../assistantUiMessages';

function msg(over: Partial<ThreadMessage> = {}): ThreadMessage {
  return {
    id: 'm1',
    content: 'hello',
    type: 'text',
    extraMetadata: {},
    sender: 'user',
    createdAt: '2026-01-01T00:00:00.000Z',
    ...over,
  };
}

function tool(over: Partial<ToolTimelineEntry> = {}): ToolTimelineEntry {
  return { id: 'call-1', name: 'shell', round: 1, seq: 0, status: 'running', ...over };
}

const GATE: PendingApproval = {
  requestId: 'appr-1',
  toolName: 'shell',
  message: 'Run shell — list the repository root',
  command: 'ls -la',
};

type ToolPart = {
  type: 'tool-call';
  toolCallId: string;
  toolName: string;
  result?: unknown;
  approval?: { id: string };
};

function toolParts(message: { content: unknown }): ToolPart[] {
  const parts = Array.isArray(message.content) ? message.content : [];
  return parts.filter((part): part is ToolPart => (part as ToolPart)?.type === 'tool-call');
}

/** Every `toolCallId` in the whole projection, duplicates included. */
function allToolCallIds(messages: ReturnType<typeof buildRuntimeMessages>): string[] {
  return messages.flatMap(message => toolParts(message).map(part => part.toolCallId));
}

describe('a parked approval in the runtime projection', () => {
  it('never hangs the gate on a call that already returned', () => {
    // An earlier `shell` came back; the gate is holding a second one whose
    // `tool_call` frame never arrived (the progress channel is bounded and
    // drops frames under load — the case `syntheticApprovalPart` exists for).
    // Matching on the tool name alone finds the FINISHED call, and a part
    // carrying a `result` is settled: assistant-ui renders no decision on it,
    // so the prompt is attached to something that cannot show it and the turn
    // parks with nothing on screen. Requiring an unsettled row is what makes
    // the projection fall through to a synthesised prompt instead.
    const messages = buildRuntimeMessages([msg()], null, {
      isRunning: true,
      pendingApproval: GATE,
      liveTimeline: [tool({ id: 'call-done', status: 'success', result: 'README.md' })],
    });

    const parts = toolParts(messages.at(-1)!);
    const gated = parts.filter(part => part.approval !== undefined);
    expect(gated).toHaveLength(1);
    expect(gated[0]?.approval?.id).toBe('appr-1');
    // A fresh, namespaced part — not the finished call.
    expect(gated[0]?.toolCallId).not.toBe('call-done');
    expect(gated[0]?.toolCallId).toBe('__openhuman_approval__:appr-1');
    expect(gated[0]?.result).toBeUndefined();
    // And the settled call is left settled rather than re-opened.
    const done = parts.find(part => part.toolCallId === 'call-done');
    expect(done?.approval).toBeUndefined();
  });

  it('still prompts when the projection reports the turn already finished', () => {
    // `chat_done` has not fired for a parked turn — it is stopped, not
    // finished — but a snapshot race can report `isRunning: false` anyway.
    // Suppressing the tail there swallows the only surface that can unblock
    // the thread, which is the whole failure this projection exists to close.
    const messages = buildRuntimeMessages([msg({ id: 'a1', sender: 'agent' })], null, {
      isRunning: false,
      pendingApproval: GATE,
      liveTimeline: [tool({ id: 'call-parked' })],
    });

    const tail = messages.find(message => message.id === STREAMING_TAIL_ID);
    expect(tail).toBeDefined();
    expect(tail?.status).toEqual({ type: 'requires-action', reason: 'interrupt' });
    expect(toolParts(tail!).find(part => part.approval?.id === 'appr-1')).toBeDefined();
  });

  it('emits each live tool call once when a gate parks on a settled turn', () => {
    // With `isRunning: false` the builder normally replays the live rows onto
    // the last agent message (the settled-live fallback), because the tail is
    // about to be suppressed. A parked gate mints the tail regardless, so both
    // would carry the same rows — and a repeated `toolCallId` THROWS inside
    // assistant-ui rather than dropping the duplicate.
    const messages = buildRuntimeMessages([msg({ id: 'a1', sender: 'agent' })], null, {
      isRunning: false,
      pendingApproval: GATE,
      liveTimeline: [tool({ id: 'call-parked' }), tool({ id: 'call-earlier', name: 'web_search' })],
    });

    const ids = allToolCallIds(messages);
    expect(ids).toEqual([...new Set(ids)]);
    expect(ids).toContain('call-parked');
  });
});
