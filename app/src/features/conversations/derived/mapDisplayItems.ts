/**
 * Phase C mapper: project the transcript-derived RPC's newest-first
 * {@link DerivedDisplayItem}s onto the **existing** settled-turn renderer
 * models, keyed by producing `requestId` — the exact shapes
 * `fetchAndHydrateTurnHistory` produces from the legacy `turn_state_history`
 * snapshot ring, so `PastTurnInsights` / `ProcessingTranscriptView` /
 * `ToolTimelineBlock` / `SubagentActivityBlock` are reused unchanged.
 *
 * Division of labour (matches how `turnTimelinesByThread` / `PastTurnInsights`
 * anchor today):
 * - **Final assistant text and user text are NOT emitted here** — they stay
 *   rendered from the thread message list (`threads_messages_list`). This
 *   mapper only produces the *process trail* (reasoning, interim narration,
 *   tool cards, sub-agent trails) that renders above a past answer.
 * - Items are grouped per `requestId` (from `turnBoundary` markers and each
 *   message's own `requestId`) so the caller can anchor a turn's trail to its
 *   first agent message by `requestId`.
 *
 * Live streaming is untouched: the caller skips the live/most-recent turn's
 * `requestId` so derived data never fights socket-fed `chatRuntimeSlice` state.
 */
import debug from 'debug';

import type {
  ProcessingTranscriptItem,
  SubagentActivity,
  SubagentToolCallEntry,
  SubagentTranscriptItem,
  ToolFailureExplanation,
  ToolTimelineEntry,
  ToolTimelineEntryStatus,
} from '../../../store/chatRuntimeSlice';
import type {
  DerivedDisplayItem,
  DerivedToolCall,
  DerivedToolCallStatus,
  DerivedToolFailure,
} from '../../../types/derivedTranscript';
import { formatTimelineEntry } from '../../../utils/toolTimelineFormatting';

const log = debug('conversations.derived.mapDisplayItems');

/** A partial assistant answer left behind by an interrupted turn. */
export interface DerivedInterruptedAnswer {
  requestId: string;
  content: string;
  thinking: string;
}

/**
 * Per-thread settled-turn process trails derived from the transcript
 * projection, ready to feed `setTurnTimelinesForThread` (timelines +
 * transcripts) and, for completeness, any interrupted partials found.
 */
export interface MappedTranscript {
  /** `requestId -> tool timeline rows` for each settled turn. */
  timelines: Record<string, ToolTimelineEntry[]>;
  /** `requestId -> processing transcript` (narration / thinking / tool ptr). */
  transcripts: Record<string, ProcessingTranscriptItem[]>;
  /**
   * Interrupted partials found in the derived data, in chronological order.
   * Exposed for a future phase; the hydration thunk deliberately leaves the
   * live/current-turn interrupted partial owned by the `turn_state` snapshot
   * path so it never fights live state.
   */
  interrupted: DerivedInterruptedAnswer[];
}

/** Options controlling {@link mapDisplayItems}. */
export interface MapDisplayItemsOptions {
  /**
   * Request ids to omit from the output entirely — the live/most-recent turn
   * (rendered from socket-fed state / the turn_state snapshot) and any turn
   * currently streaming. Their trails must not double-render against the live
   * anchor.
   */
  skipRequestIds?: ReadonlySet<string>;
}

/** Map the Rust `ToolCallStatus` onto the timeline status vocabulary. A settled
 *  turn whose tool row is still `running` had no result line paired (the turn
 *  was interrupted before completion) — settle it to `cancelled` (terminal,
 *  muted, non-pulsing), mirroring `settleOrphanedTimelineEntry`. */
function timelineStatusFromDerived(status: DerivedToolCallStatus): ToolTimelineEntryStatus {
  switch (status) {
    case 'success':
      return 'success';
    case 'error':
      return 'error';
    case 'running':
    default:
      return 'cancelled';
  }
}

/** Same mapping for a sub-agent child tool call. */
function subagentToolStatus(status: DerivedToolCallStatus): ToolTimelineEntryStatus {
  return timelineStatusFromDerived(status);
}

/**
 * Expand the minimal wire {@link DerivedToolFailure} into the richer
 * {@link ToolFailureExplanation} the `ToolFailureLines` renderer consumes,
 * matching `turn_state`'s `PersistedToolFailure` shape. The projection only
 * records that a call failed plus an optional short reason, so we synthesise an
 * unlocalized `Unknown`/`Recoverable` explanation whose `causePlain` carries the
 * captured detail (or the tool's error output) — `ToolFailureLines` falls back
 * to `causePlain`/`nextAction` for unrecognised classes.
 */
function toFailureExplanation(
  failure: DerivedToolFailure | undefined,
  result: string | undefined
): ToolFailureExplanation | undefined {
  if (!failure) return undefined;
  const causePlain = failure.detail?.trim() || result?.trim() || 'The tool reported an error.';
  return {
    class: 'Unknown',
    category: 'Recoverable',
    recoverable: true,
    causePlain,
    nextAction: 'Review the tool output and try again.',
  };
}

function stringifyArgs(args: unknown): string | undefined {
  if (args === undefined || args === null) return undefined;
  if (typeof args === 'string') return args;
  try {
    return JSON.stringify(args);
  } catch {
    return undefined;
  }
}

/**
 * Build a {@link SubagentActivity} from a `subagent` display item's nested
 * items. The nested vocabulary (reasoning / assistantMessage / toolCall)
 * projects onto the sub-agent transcript (`thinking` / `text` / `tool`) plus a
 * flat `toolCalls` list — exactly what `SubagentActivityBlock` reads.
 */
function buildSubagentActivity(id: string, items: DerivedDisplayItem[]): SubagentActivity {
  const toolCalls: SubagentToolCallEntry[] = [];
  const transcript: SubagentTranscriptItem[] = [];

  for (const item of items) {
    switch (item.kind) {
      case 'reasoning':
        if (item.text.trim()) {
          transcript.push({ kind: 'thinking', text: item.text });
        }
        break;
      case 'assistantMessage':
        // A sub-agent's own answer text is part of its inline trail (there is
        // no separate message bubble for a delegated worker).
        if (item.content.trim()) {
          transcript.push({ kind: 'text', iteration: item.iteration, text: item.content });
        }
        break;
      case 'toolCall': {
        const status = subagentToolStatus(item.status);
        toolCalls.push({
          callId: item.callId,
          toolName: item.name,
          status,
          args: item.args,
          result: item.result,
        });
        transcript.push({
          kind: 'tool',
          callId: item.callId,
          toolName: item.name,
          status,
          args: item.args,
          result: item.result,
        });
        break;
      }
      // Nested sub-agents, turn boundaries, interrupted partials and compaction
      // markers do not surface inside a sub-agent block — the projection nests
      // deeper sub-agents as their own top-level items.
      default:
        break;
    }
  }

  return { taskId: id, agentId: id, status: 'completed', toolCalls, transcript };
}

/** Mutable per-turn accumulator. */
interface TurnAccumulator {
  entries: ToolTimelineEntry[];
  transcript: ProcessingTranscriptItem[];
  seq: number;
  round: number;
}

function ensureTurn(turns: Map<string, TurnAccumulator>, requestId: string): TurnAccumulator {
  let turn = turns.get(requestId);
  if (!turn) {
    turn = { entries: [], transcript: [], seq: 0, round: 0 };
    turns.set(requestId, turn);
  }
  return turn;
}

/**
 * Map a newest-first page of derived display items into per-`requestId`
 * settled-turn trails. Returns empty maps when nothing anchors to a `requestId`
 * (e.g. a legacy thread whose lines carry no `requestId`).
 */
export function mapDisplayItems(
  items: DerivedDisplayItem[],
  options: MapDisplayItemsOptions = {}
): MappedTranscript {
  const skip = options.skipRequestIds ?? new Set<string>();
  const turns = new Map<string, TurnAccumulator>();
  const interrupted: DerivedInterruptedAnswer[] = [];

  // The RPC returns items newest-first; walk chronologically so `seq` reflects
  // issue order and turn boundaries advance forward.
  const chronological = [...items].reverse();
  let currentRequestId: string | undefined;

  const skipped = new Set<string>();

  for (const item of chronological) {
    switch (item.kind) {
      case 'turnBoundary':
        currentRequestId = item.requestId;
        break;

      case 'userMessage':
        // User text renders from the thread message list; only advance the
        // turn cursor so following items anchor to this turn.
        if (item.requestId) currentRequestId = item.requestId;
        break;

      case 'assistantMessage': {
        if (item.requestId) currentRequestId = item.requestId;
        if (item.iteration !== undefined && currentRequestId && !skip.has(currentRequestId)) {
          ensureTurn(turns, currentRequestId).round = item.iteration;
        }
        // Final (non-interim) answer renders from the thread message; only
        // interim narration belongs to the process trail.
        if (!item.interim) break;
        if (!currentRequestId || skip.has(currentRequestId)) {
          if (currentRequestId) skipped.add(currentRequestId);
          break;
        }
        const text = item.content.trim();
        if (!text) break;
        const turn = ensureTurn(turns, currentRequestId);
        turn.transcript.push({
          kind: 'narration',
          round: turn.round,
          seq: turn.seq++,
          text: item.content,
        });
        break;
      }

      case 'reasoning': {
        if (!currentRequestId || skip.has(currentRequestId)) {
          if (currentRequestId) skipped.add(currentRequestId);
          break;
        }
        if (!item.text.trim()) break;
        const turn = ensureTurn(turns, currentRequestId);
        turn.transcript.push({
          kind: 'thinking',
          round: turn.round,
          seq: turn.seq++,
          text: item.text,
        });
        break;
      }

      case 'toolCall': {
        if (!currentRequestId || skip.has(currentRequestId)) {
          if (currentRequestId) skipped.add(currentRequestId);
          break;
        }
        const turn = ensureTurn(turns, currentRequestId);
        pushToolCall(turn, item);
        break;
      }

      case 'subagent': {
        // Anchor to the turn the sub-agent was spawned in (core-derived
        // `requestId`), not the current cursor — sub-agent items are appended
        // after all root items, so the cursor is the last turn by then.
        const anchorRequestId = item.requestId ?? currentRequestId;
        if (!anchorRequestId || skip.has(anchorRequestId)) {
          if (anchorRequestId) skipped.add(anchorRequestId);
          break;
        }
        const turn = ensureTurn(turns, anchorRequestId);
        const activity = buildSubagentActivity(item.id, item.items);
        turn.entries.push({
          id: `subagent:${item.id}`,
          name: `subagent:${item.id}`,
          round: turn.round,
          seq: turn.seq++,
          status: 'success',
          subagent: activity,
        });
        break;
      }

      case 'interruptedPartial':
        if (currentRequestId) {
          interrupted.push({
            requestId: currentRequestId,
            content: item.text,
            thinking: item.thinking ?? '',
          });
        }
        break;

      // Compaction markers have no settled-turn renderer — drop them.
      case 'compaction':
      default:
        break;
    }
  }

  const timelines: Record<string, ToolTimelineEntry[]> = {};
  const transcripts: Record<string, ProcessingTranscriptItem[]> = {};
  for (const [requestId, turn] of turns) {
    if (turn.entries.length > 0) timelines[requestId] = turn.entries;
    if (turn.transcript.length > 0) transcripts[requestId] = turn.transcript;
  }

  log(
    'mapped turns=%d timelines=%d transcripts=%d interrupted=%d skipped=%d',
    turns.size,
    Object.keys(timelines).length,
    Object.keys(transcripts).length,
    interrupted.length,
    skipped.size
  );

  return { timelines, transcripts, interrupted };
}

/** Push a tool-call display item as both a timeline entry and a transcript
 *  pointer sharing one `seq`, so the tool row orders consistently among the
 *  turn's narration / thinking. */
function pushToolCall(turn: TurnAccumulator, item: DerivedToolCall): void {
  const seq = turn.seq++;
  const entry: ToolTimelineEntry = {
    id: item.callId,
    name: item.name,
    round: turn.round,
    seq,
    status: timelineStatusFromDerived(item.status),
    argsBuffer: stringifyArgs(item.args),
    result: item.result,
  };
  // A failed tool renders its "why / next" explanation via `ToolFailureLines`.
  const failure = toFailureExplanation(item.failure, item.result);
  if (failure) entry.failure = failure;
  // Derive the human label + detail from tool name + args (the same TS
  // formatter the live path runs), so settled rows carry `displayName`/`detail`
  // at parity with `turn_state` rows instead of being unlabelled.
  const formatted = formatTimelineEntry(entry);
  entry.displayName = formatted.title;
  if (formatted.detail !== undefined) entry.detail = formatted.detail;
  turn.entries.push(entry);
  turn.transcript.push({ kind: 'toolCall', round: turn.round, seq, callId: item.callId });
}
