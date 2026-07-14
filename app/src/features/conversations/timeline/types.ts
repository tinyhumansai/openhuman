/**
 * Unified timeline model for the conversations panel.
 *
 * `threadSlice` (durable messages) and `chatRuntimeSlice` (live tool timeline,
 * streaming previews) remain the sources of truth; `selectTimelineForThread`
 * (see `./selectors.ts`) projects them into one ordered `TimelineItem[]`. This
 * is a *projection*, not a new slice — no big-bang migration, and each phase of
 * the refactor can land independently (see
 * `docs/plans/conversations-timeline-refactor.md`).
 *
 * Every item carries a stable `id`, its `turnId` (the request that produced it;
 * `'legacy'` for pre-migration turns whose messages carry no `requestId`), a
 * per-turn ordering `seq`, and its `threadId`. Ordering within a turn is by
 * `seq`; turns order by the position of their first message in the thread.
 */
import type { ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import type { ThreadMessage } from '../../../types/thread';

/**
 * Turn id used for every item on a thread whose messages predate per-turn
 * `requestId` stamping (Phase 4). Until then the whole thread is one turn, which
 * reproduces today's single-anchor timeline behavior.
 */
export const LEGACY_TURN_ID = 'legacy';

/** Normalised tool status for the timeline (`success` → `ok`). */
export type TimelineToolStatus = 'running' | 'ok' | 'error' | 'awaiting_user' | 'cancelled';

export interface TimelineItemBase {
  /** Stable id: message id, or the runtime row id (`ToolTimelineEntry.id`). */
  id: string;
  /** Request that produced this item; `LEGACY_TURN_ID` before requestId stamping. */
  turnId: string;
  /** Ordering within the turn. Reducer-assigned now; backend-stamped in Phase 4. */
  seq: number;
  threadId: string;
}

export type TimelineItem = TimelineItemBase &
  (
    | { kind: 'userMessage'; message: ThreadMessage }
    | { kind: 'assistantMessage'; message: ThreadMessage; interim: boolean }
    | {
        /** Ephemeral live-turn streaming tail (primary or a forked branch). */
        kind: 'streamingText';
        text: string;
        thinking?: string;
        /** `requestId` of the stream; distinguishes the primary from branches. */
        streamId?: string;
        /** True for a `parallelStreamsByThread` forked branch. */
        branch: boolean;
      }
    | { kind: 'reasoning'; text: string; settled: boolean }
    | {
        kind: 'toolCall';
        /** The underlying runtime row (rendered via `ToolTimelineBlock`). */
        entry: ToolTimelineEntry;
        callId: string;
        name: string;
        status: TimelineToolStatus;
        round: number;
      }
    | {
        /** A `subagent:*` tool row, rendered nested but stored flat. */
        kind: 'subagentActivity';
        entry: ToolTimelineEntry;
        taskId: string;
        callId: string;
        name: string;
        status: TimelineToolStatus;
        round: number;
      }
  );

export type TimelineItemKind = TimelineItem['kind'];

/** The "agent process" kinds that `hideAgentInsights` suppresses. */
export const AGENT_INSIGHT_KINDS: readonly TimelineItemKind[] = [
  'toolCall',
  'subagentActivity',
  'reasoning',
];

/** A contiguous group of items sharing a `turnId`, in render order. */
export interface TimelineTurn {
  turnId: string;
  items: TimelineItem[];
}

/** Map `ToolTimelineEntry.status` onto the normalised timeline status. */
export function toTimelineToolStatus(status: ToolTimelineEntry['status']): TimelineToolStatus {
  return status === 'success' ? 'ok' : status;
}

/** A `subagent:*` row carries live/persisted sub-agent activity. */
export function isSubagentEntry(entry: ToolTimelineEntry): boolean {
  return entry.name.startsWith('subagent:');
}
