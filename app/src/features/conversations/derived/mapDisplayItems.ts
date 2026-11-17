/**
 * Phase C mapper: project the transcript-derived RPC's newest-first
 * {@link DerivedDisplayItem}s onto the **existing** settled-turn renderer
 * models, keyed by producing `requestId` — the exact shapes
 * `fetchAndHydrateTurnHistory` produces from the legacy `turn_state_history`
 * snapshot ring, so `ProcessingTranscriptView` / assistant-ui tool and
 * delegation cards are reused unchanged.
 *
 * Division of labour (matches how `turnTimelinesByThread` anchors today):
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
  DerivedSubagent,
  DerivedSubagentStatus,
  DerivedToolCall,
  DerivedToolCallStatus,
  DerivedToolFailure,
} from '../../../types/derivedTranscript';
import { formatTimelineEntry } from '../../../utils/toolTimelineFormatting';

const log = debug('conversations.derived.mapDisplayItems');

/** A partial assistant answer left behind by an interrupted turn. */
interface DerivedInterruptedAnswer {
  requestId: string;
  content: string;
  thinking: string;
}

/**
 * Per-thread settled-turn process trails derived from the transcript
 * projection, ready to feed `setTurnTimelinesForThread` (timelines +
 * transcripts) and, for completeness, any interrupted partials found.
 */
interface MappedTranscript {
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
interface MapDisplayItemsOptions {
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
 * flat `toolCalls` list — exactly what the assistant-ui delegation card reads.
 */
function buildSubagentActivity(item: DerivedSubagent): SubagentActivity {
  const { items } = item;
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

  return {
    taskId: item.taskId ?? item.id,
    agentId: item.agentId ?? item.id,
    status: subagentActivityStatus(item.status),
    toolCalls,
    transcript,
  };
}

/** The delegation card's activity status for a settled sub-agent run. An
 *  older core sent no status; it only ever reported finished runs. */
function subagentActivityStatus(status: DerivedSubagentStatus | undefined): string {
  switch (status) {
    case 'failed':
      return 'failed';
    case 'incomplete':
    case 'interrupted':
    case 'running':
      return status;
    case 'completed':
    default:
      return 'completed';
  }
}

/** The timeline row status for a settled sub-agent run — the same settling
 *  rule as a tool row: a run with no terminal record is `cancelled`. */
function subagentEntryStatus(status: DerivedSubagentStatus | undefined): ToolTimelineEntryStatus {
  switch (status) {
    case 'failed':
      return 'error';
    // Stopped without a complete answer (timeout or exhausted budget): not a
    // success, but not a hard failure either, so it settles like a cancel.
    case 'incomplete':
    case 'interrupted':
    case 'running':
      return 'cancelled';
    case 'completed':
    default:
      return 'success';
  }
}

/** Mutable per-turn accumulator. */
interface TurnAccumulator {
  entries: ToolTimelineEntry[];
  transcript: ProcessingTranscriptItem[];
  seq: number;
  round: number;
  /** Row ids already minted for this turn — see {@link uniqueCallId}. */
  callIds: Set<string>;
}

function ensureTurn(turns: Map<string, TurnAccumulator>, requestId: string): TurnAccumulator {
  let turn = turns.get(requestId);
  if (!turn) {
    turn = { entries: [], transcript: [], seq: 0, round: 0, callIds: new Set() };
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
        // Reasoning is projected *before* the message of its step, so the
        // round must come from the reasoning itself — waiting for the
        // following assistantMessage filed it under the previous step.
        if (item.iteration !== undefined) turn.round = item.iteration;
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
        // A step with no visible narration emits no assistantMessage, so the
        // call carries its own step number.
        if (item.iteration !== undefined) turn.round = item.iteration;
        pushToolCall(turn, item);
        break;
      }

      case 'subagent': {
        // The core places a sub-agent right after its spawning call (or at the
        // end of its turn), so it arrives in order; its own `requestId` still
        // wins over the cursor for payloads from an older core, which
        // appended every sub-agent after all root items.
        const anchorRequestId = item.requestId ?? currentRequestId;
        if (!anchorRequestId || skip.has(anchorRequestId)) {
          if (anchorRequestId) skipped.add(anchorRequestId);
          break;
        }
        const turn = ensureTurn(turns, anchorRequestId);
        const activity = buildSubagentActivity(item);
        const agentId = activity.agentId;
        placeSubagentRow(
          turn,
          {
            // `item.id` is unique per run (task id) on a current core; an older
            // core sends the agent name, so it is still disambiguated per turn.
            id: uniqueSubagentRowId(turn, item.id),
            name: `subagent:${agentId}`,
            round: turn.round,
            seq: 0,
            status: subagentEntryStatus(item.status),
            subagent: activity,
          },
          item.callId
        );
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

/** Tool names that open a delegation — mirrors `findPendingDelegationContext`. */
function isDelegationToolName(name: string): boolean {
  return (
    name === 'spawn_subagent' || name === 'spawn_async_subagent' || name.startsWith('delegate_')
  );
}

/** `subagent:<id>`, disambiguated when one turn delegates to the same agent twice. */
function uniqueSubagentRowId(turn: TurnAccumulator, agentId: string): string {
  const base = `subagent:${agentId}`;
  let id = base;
  for (let n = 2; turn.callIds.has(id); n += 1) id = `${base}#${n}`;
  turn.callIds.add(id);
  return id;
}

/**
 * Put a sub-agent row where the live stream puts it: in the slot of the call
 * that spawned it.
 *
 * The projection nests sub-agents as sibling items after every root item (the
 * transcript records no link from a delegation call to the child's file), so
 * appending them gave every reopened turn a different shape from the one the
 * user watched stream: the spawn call rendered as a plain tool card and the
 * delegation card sat at the bottom of the turn. Children are ordered by spawn
 * time, and so are the turn's delegation calls, so the Nth child takes the Nth
 * not-yet-claimed delegation row — its `seq` and its transcript pointer —
 * exactly as `subagentSpawned` does live. With no such row (an older
 * transcript, or a spawn outside this turn's page) it is appended as before.
 */
function placeSubagentRow(
  turn: TurnAccumulator,
  row: ToolTimelineEntry,
  spawnCallId: string | undefined
): void {
  // A current core names the spawning call and has already placed the child
  // right after it in the stream. An older core names none and appends every
  // child after all root items, so the Nth child pairs with the Nth unclaimed
  // delegation row.
  const spawnIdx = spawnCallId
    ? turn.entries.findIndex(entry => entry.subagent === undefined && entry.id === spawnCallId)
    : turn.entries.findIndex(
        entry => entry.subagent === undefined && isDelegationToolName(entry.name)
      );
  const spawn = spawnIdx >= 0 ? turn.entries[spawnIdx] : undefined;
  // Only a delegation-shaped call is folded into its card, exactly the calls
  // `findPendingDelegationContext` folds live; any other spawning call (an
  // agent exposed as a tool) stays a card of its own with the child after it.
  if (!spawn || !isDelegationToolName(spawn.name)) {
    turn.entries.push({ ...row, seq: turn.seq++ });
    return;
  }
  turn.entries[spawnIdx] = {
    ...row,
    seq: spawn.seq,
    detail: spawn.detail,
    sourceToolName: spawn.name,
    subagent: row.subagent ? { ...row.subagent, prompt: spawn.detail } : row.subagent,
  };
  const pointer = turn.transcript.find(
    item => item.kind === 'toolCall' && item.callId === spawn.id
  );
  if (pointer && pointer.kind === 'toolCall') pointer.callId = row.id;
}

/**
 * A row id that is unique within the turn.
 *
 * `callId` comes off the persisted session transcript, so it is whatever the
 * provider wrote — and a provider that emits tool calls without ids writes the
 * empty string for every one of them. Two such calls in a turn used to collapse
 * onto one id, which broke twice over: the timeline map keyed by id kept only
 * the last, so the earlier call vanished from the UI; and the transcript then
 * held two pointers to that one row, so the assistant-ui projection emitted the
 * same tool part twice. assistant-ui keys parts as `toolCallId-${id}` and
 * *throws* on a repeat ("Duplicate key toolCallId- in useResources"), which
 * takes the whole thread render down on load.
 *
 * The fallback is positional, so it is stable across re-derivations of the same
 * transcript — the id has to survive a remount, not just be unique once.
 */
function uniqueCallId(turn: TurnAccumulator, callId: string, seq: number): string {
  const base = callId || `derived:${turn.round}:${seq}`;
  if (!turn.callIds.has(base)) {
    turn.callIds.add(base);
    return base;
  }
  const disambiguated = `${base}#${seq}`;
  turn.callIds.add(disambiguated);
  return disambiguated;
}

/** Push a tool-call display item as both a timeline entry and a transcript
 *  pointer sharing one `seq`, so the tool row orders consistently among the
 *  turn's narration / thinking. */
function pushToolCall(turn: TurnAccumulator, item: DerivedToolCall): void {
  // The derived transcript can retain both the initial and terminal record for
  // one call. They describe one UI step, not two calls: merge the later record
  // into the existing row instead of manufacturing a `#seq` duplicate.
  if (item.callId) {
    const existing = turn.entries.find(entry => entry.id === item.callId);
    const status = timelineStatusFromDerived(item.status);
    // A replayed in-flight record is represented as `cancelled` by the
    // settled mapper; a later terminal record for that same call supersedes
    // it. Repeated completed IDs remain distinct calls (and stay disambiguated
    // below), preserving parallel/retry history.
    if (existing && (existing.status === 'cancelled' || status === 'cancelled')) {
      existing.status = status === 'cancelled' ? existing.status : status;
      existing.argsBuffer = stringifyArgs(item.args) ?? existing.argsBuffer;
      existing.result = item.result ?? existing.result;
      const failure = toFailureExplanation(item.failure, item.result);
      if (failure) existing.failure = failure;
      return;
    }
  }
  const seq = turn.seq++;
  const callId = uniqueCallId(turn, item.callId, seq);
  const entry: ToolTimelineEntry = {
    id: callId,
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
  // Derive the detail from tool name + args (the same registry the live path
  // runs). The title is *not* baked into `displayName`: that field carries
  // only a server label, and a baked title froze its tense at "Reading file"
  // on a finished row. Surfaces resolve the title at render time.
  const formatted = formatTimelineEntry(entry);
  if (entry.detail === undefined && formatted.detail !== undefined) entry.detail = formatted.detail;
  turn.entries.push(entry);
  turn.transcript.push({ kind: 'toolCall', round: turn.round, seq, callId });
}
