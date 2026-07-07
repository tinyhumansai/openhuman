/**
 * Projection of the durable + live conversation state into one ordered
 * `TimelineItem[]` (see `./types.ts` and
 * `docs/plans/conversations-timeline-refactor.md`).
 *
 * `buildThreadTimeline` is a pure function of its inputs — fully unit-testable
 * without a store. `selectTimelineForThread` is the memoized Redux selector that
 * feeds it from state; reselect's weakMap memoization keys on the input slices
 * so switching threads does not thrash the cache.
 *
 * ## Ordering (reproduces today's single-anchor behavior)
 *
 * Messages render in stored append order, filtered to drop
 * `extraMetadata.hidden`. The single tool timeline is anchored *after the last
 * user message* — matching the current positional hack
 * (`Conversations.tsx` `lastUserMessageId` at ~L1756 / L2533). Threads with no
 * user message (proactive-only) place the process items at the end, matching the
 * L2669 fallback. Ephemeral streaming previews (primary + forked branches)
 * always trail the durable items.
 *
 * Until Phase 4 stamps `requestId` on messages, every item belongs to the
 * `LEGACY_TURN_ID` turn, so the projection is behaviorally identical to today.
 */
import { createSelector } from '@reduxjs/toolkit';

import type { StreamingAssistantState, ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import type { RootState } from '../../../store/index';
import type { ThreadMessage } from '../../../types/thread';
import {
  isSubagentEntry,
  LEGACY_TURN_ID,
  type TimelineItem,
  type TimelineTurn,
  toTimelineToolStatus,
} from './types';

export interface BuildThreadTimelineInput {
  threadId: string;
  messages: ThreadMessage[];
  toolTimeline: ToolTimelineEntry[];
  /** Primary streaming preview for the thread, or null. */
  streaming: StreamingAssistantState | null;
  /** Forked/parallel branch streams for the thread. */
  parallelStreams: StreamingAssistantState[];
  /** When true, the process kinds (tool/subagent rows) are omitted. */
  hideAgentInsights: boolean;
}

function isHidden(message: ThreadMessage): boolean {
  return Boolean(message.extraMetadata?.hidden);
}

/**
 * The turn a message belongs to. Assistant/user messages produced after the
 * Phase 4 rollout carry `extraMetadata.requestId`; older messages have none and
 * fall back to the single `legacy` turn (today's single-anchor behavior).
 */
function messageTurnId(message: ThreadMessage): string {
  const requestId = message.extraMetadata?.requestId;
  return typeof requestId === 'string' && requestId.length > 0 ? requestId : LEGACY_TURN_ID;
}

/** Map one runtime tool-timeline row onto a `toolCall`/`subagentActivity` item. */
function toProcessItem(entry: ToolTimelineEntry, threadId: string, seq: number): TimelineItem {
  const base = {
    id: entry.id,
    turnId: LEGACY_TURN_ID,
    seq,
    threadId,
    callId: entry.id,
    name: entry.name,
    status: toTimelineToolStatus(entry.status),
    round: entry.round,
    entry,
  } as const;
  if (isSubagentEntry(entry) && entry.subagent) {
    return { ...base, kind: 'subagentActivity', taskId: entry.subagent.taskId };
  }
  return { ...base, kind: 'toolCall' };
}

/**
 * Pure projection: compose messages + tool timeline + streaming previews into
 * one ordered `TimelineItem[]`. See the module doc for ordering rules.
 */
export function buildThreadTimeline(input: BuildThreadTimelineInput): TimelineItem[] {
  const { threadId, messages, toolTimeline, streaming, parallelStreams, hideAgentInsights } = input;

  const visible = messages.filter(m => !isHidden(m));
  // Anchor id: the *last* user message, mirroring the current positional hack.
  let lastUserMessageId: string | undefined;
  for (let i = visible.length - 1; i >= 0; i -= 1) {
    if (visible[i].sender === 'user') {
      lastUserMessageId = visible[i].id;
      break;
    }
  }

  const items: TimelineItem[] = [];
  let seq = 0;

  const pushProcessItems = () => {
    if (hideAgentInsights) return;
    for (const entry of toolTimeline) {
      items.push(toProcessItem(entry, threadId, seq));
      seq += 1;
    }
  };

  for (const message of visible) {
    if (message.sender === 'user') {
      items.push({
        kind: 'userMessage',
        id: message.id,
        turnId: messageTurnId(message),
        seq,
        threadId,
        message,
      });
    } else {
      items.push({
        kind: 'assistantMessage',
        id: message.id,
        turnId: messageTurnId(message),
        seq,
        threadId,
        message,
        // Durable persisted messages are never interim narration; interim
        // (`chat_interim`) items are live-only and folded in during streaming.
        interim: false,
      });
    }
    seq += 1;
    // Anchor the process block immediately after the last user message so it
    // reads above the agent's answer for that turn.
    if (message.id === lastUserMessageId) {
      pushProcessItems();
    }
  }

  // Proactive / no-user-message threads: process items have no anchor above, so
  // they trail the messages (mirrors the L2669 fallback).
  if (!lastUserMessageId) {
    pushProcessItems();
  }

  // Ephemeral streaming previews always trail the durable items (they belong to
  // the live turn). `thinking` rides along on the primary stream; forked
  // branches render content only, matching today.
  if (streaming && (streaming.content.length > 0 || streaming.thinking.length > 0)) {
    items.push({
      kind: 'streamingText',
      id: `stream:${streaming.requestId}`,
      turnId: LEGACY_TURN_ID,
      seq,
      threadId,
      text: streaming.content,
      thinking: streaming.thinking.length > 0 ? streaming.thinking : undefined,
      streamId: streaming.requestId,
      branch: false,
    });
    seq += 1;
  }
  for (const branch of parallelStreams) {
    if (branch.content.length === 0 && branch.thinking.length === 0) continue;
    items.push({
      kind: 'streamingText',
      id: `stream:branch:${branch.requestId}`,
      turnId: LEGACY_TURN_ID,
      seq,
      threadId,
      text: branch.content,
      streamId: branch.requestId,
      branch: true,
    });
    seq += 1;
  }

  return items;
}

/** Split a flat, ordered timeline into contiguous per-turn groups. */
export function groupTimelineIntoTurns(items: TimelineItem[]): TimelineTurn[] {
  const turns: TimelineTurn[] = [];
  for (const item of items) {
    const last = turns[turns.length - 1];
    if (last && last.turnId === item.turnId) {
      last.items.push(item);
    } else {
      turns.push({ turnId: item.turnId, items: [item] });
    }
  }
  return turns;
}

const EMPTY_MESSAGES: ThreadMessage[] = [];
const EMPTY_TIMELINE: ToolTimelineEntry[] = [];

/**
 * Memoized projection selector. Pass the thread id as the second arg:
 * `selectTimelineForThread(state, threadId)`.
 */
export const selectTimelineForThread = createSelector(
  [
    (state: RootState, threadId: string) =>
      state.thread.messagesByThreadId[threadId] ?? EMPTY_MESSAGES,
    (state: RootState, threadId: string) =>
      state.chatRuntime.toolTimelineByThread[threadId] ?? EMPTY_TIMELINE,
    (state: RootState, threadId: string) =>
      state.chatRuntime.streamingAssistantByThread[threadId] ?? null,
    (state: RootState, threadId: string) => state.chatRuntime.parallelStreamsByThread[threadId],
    (state: RootState) => state.theme?.hideAgentInsights ?? false,
    (_state: RootState, threadId: string) => threadId,
  ],
  (messages, toolTimeline, streaming, parallelMap, hideAgentInsights, threadId) =>
    buildThreadTimeline({
      threadId,
      messages,
      toolTimeline,
      streaming,
      parallelStreams: parallelMap ? Object.values(parallelMap) : [],
      hideAgentInsights,
    })
);
