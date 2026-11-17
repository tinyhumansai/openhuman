import {
  type ThreadMessage as AuiThreadMessage,
  fromThreadMessageLike,
  type ThreadAssistantMessagePart,
  type ThreadMessageLike,
  type ThreadUserMessagePart,
  type ToolApprovalOption,
  type ToolCallMessagePart,
} from '@assistant-ui/react';

import { parseBubbleSegments } from '../features/conversations/utils/format';
import { parseAttachmentReferences, parseMessageImages } from '../lib/attachments';
import { unwrapToolCallEnvelope } from '../lib/chat/toolCallEnvelope';
import type { ChatCitation } from '../services/chatService';
import {
  isActiveTimelineStatus,
  type PendingApproval,
  type ProcessingTranscriptItem,
  type StreamingAssistantState,
  type SubagentActivity,
  type SubagentTranscriptItem,
  type ToolTimelineEntry,
} from '../store/chatRuntimeSlice';
import {
  CHAT_ERROR_METADATA_KEY,
  FEEDBACK_METADATA_KEY,
  FEEDBACK_ROW_IDS_METADATA_KEY,
  type MessageFeedback,
  TIMING_METADATA_KEY,
} from '../store/threadSlice';
import type { ThreadMessage } from '../types/thread';
import { extractAgentSources } from '../utils/toolTimelineFormatting';

/**
 * Redux -> assistant-ui message mapping.
 *
 * assistant-ui is adopted as a *runtime* (semantics + API), never as a store:
 * `chatRuntimeSlice` and `threadSlice` remain the single source of truth for
 * messages, streaming, tool state, queueing and persistence. Everything here is
 * a pure, read-only projection of that state onto the shape the runtime wants.
 * Nothing in this module writes.
 *
 * The one property that matters for performance is stated as a test, not a
 * comment: converting the transcript while a token streams must not re-convert
 * the settled messages above the live tail. `ChatThreadView.renderPerf.test.tsx`
 * pins the equivalent property for the render tree; `assistantUiMessages.test.ts`
 * pins it for this projection.
 */

type ConversionCacheEntry = {
  timeline: readonly ToolTimelineEntry[];
  transcript: readonly ProcessingTranscriptItem[];
  converted: ThreadMessageLike;
};

/**
 * Cache keyed on the source message and its persisted process arrays. Socket
 * tokens only replace the live tail, so a settled message converts exactly
 * once while its transcript/timeline identities remain stable.
 */
const conversionCache = new WeakMap<ThreadMessage, ConversionCacheEntry>();

const EMPTY_TIMELINE: readonly ToolTimelineEntry[] = [];
const EMPTY_TRANSCRIPT: readonly ProcessingTranscriptItem[] = [];
const EMPTY_CITATIONS: readonly ChatCitation[] = [];

const RECOVERED_TOOL_NAMES_KEY = 'assistantUiToolNames';

/**
 * The rating persisted on a message, when it is one of the two values the
 * runtime accepts.
 *
 * `extraMetadata` is untyped JSON from disk, so this narrows rather than casts:
 * a stale or hand-edited value must render as "unrated" instead of reaching the
 * runtime as a bad `submittedFeedback`.
 */
function persistedFeedback(msg: ThreadMessage): MessageFeedback | undefined {
  const value = msg.extraMetadata?.[FEEDBACK_METADATA_KEY];
  return value === 'positive' || value === 'negative' ? value : undefined;
}

/** Synthetic id for the live streaming tail. Stable so React reconciles it. */
export const STREAMING_TAIL_ID = '__openhuman_streaming_tail__';

/** Use the core reply identity from the first token through final persistence. */
export const streamingMessageId = (requestId?: string) =>
  requestId ? `agent:${requestId}` : STREAMING_TAIL_ID;

/**
 * Convert one persisted message.
 *
 * Agent content is passed through `unwrapToolCallEnvelope` for the same reason
 * the transcript renderer does it: a `{content, tool_calls}` provider envelope
 * must never reach a rendered surface as raw JSON. Tool *activity* is not
 * projected as assistant-ui tool-call parts — it lives in the far richer
 * `toolTimelineByThread` projection that `ToolTimelineBlock` renders, and
 * duplicating it here would paint every tool twice.
 */
function jsonObject(value: unknown): Record<string, never> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
  try {
    return JSON.parse(JSON.stringify(value)) as Record<string, never>;
  } catch {
    return {};
  }
}

function toolArgs(entry: ToolTimelineEntry): Record<string, never> {
  if (!entry.argsBuffer) return {};
  try {
    return jsonObject(JSON.parse(entry.argsBuffer));
  } catch {
    return { raw: entry.argsBuffer } as never;
  }
}

/**
 * The `result` payload for a settled non-sub-agent tool part.
 *
 * assistant-ui's tool-call part has no status field, so a terminal status has
 * to travel inside `result` or not at all. It did not travel: the adapter fell
 * back to `result !== undefined`, which reads as success, and a failed or
 * cancelled tool rendered with a "done" label and a check.
 *
 * A tool that produced no output already reported `{ status, failure }` here,
 * so only the failed-*with*-output case needed a shape — `value` carries the
 * real output beside the status, and {@link isToolStatusEnvelope} unwraps it.
 * The success path is byte-identical to before, deliberately: every reader of
 * a successful result keeps seeing exactly what it saw.
 */
function toolResultPayload(entry: ToolTimelineEntry): unknown {
  const terminalFailure = entry.status === 'error' || entry.status === 'cancelled';
  if (!terminalFailure) return entry.result ?? { status: entry.status, failure: entry.failure };
  return {
    status: entry.status,
    failure: entry.failure,
    ...(entry.result !== undefined ? { value: entry.result } : {}),
  };
}

/**
 * Presentation data that rides a tool part's `artifact`.
 *
 * assistant-ui's tool-call part has no slot for a display label, a duration
 * or a structured result, and this adapter used to drop all three, so the
 * chat card fell back to guessing a label from the tool name and arguments.
 * `artifact` is the part's UI-only field, which is exactly this.
 */
export interface OpenHumanToolArtifact {
  kind: 'openhuman-tool';
  /** Server label, for dynamic tools the client registry cannot describe. */
  displayName?: string;
  detail?: string;
  elapsedMs?: number;
  structured?: unknown;
}

export function readOpenHumanToolArtifact(value: unknown): OpenHumanToolArtifact | undefined {
  if (!value || typeof value !== 'object') return undefined;
  return (value as { kind?: unknown }).kind === 'openhuman-tool'
    ? (value as OpenHumanToolArtifact)
    : undefined;
}

function toolArtifact(entry: ToolTimelineEntry): OpenHumanToolArtifact | undefined {
  const artifact: OpenHumanToolArtifact = {
    kind: 'openhuman-tool',
    ...(entry.displayName ? { displayName: entry.displayName } : {}),
    ...(entry.detail ? { detail: entry.detail } : {}),
    ...(entry.elapsedMs !== undefined ? { elapsedMs: entry.elapsedMs } : {}),
    ...(entry.structured !== undefined ? { structured: entry.structured } : {}),
  };
  return Object.keys(artifact).length > 1 ? artifact : undefined;
}

/**
 * Fold a live subagent activity's synthetic timeline row into the ORIGINAL
 * spawn/delegate tool-call row it belongs to, when the core told us which one
 * that is (`SubagentActivity.parentCallId`, from `subagent_spawned.subagent.
 * parent_call_id`).
 *
 * Before `parent_call_id` existed, the reducer had to guess which running
 * `spawn_subagent`/`delegate_*` row started a delegation and splice it out of
 * the timeline (`findPendingDelegationContext`) so only the subagent's own
 * synthetic row survived. With the real id, the two rows can both stay in
 * `chatRuntimeSlice` (simpler, and the delegation's OWN args/timing are still
 * on the spawn row) — the substitution happens here, once, at render time:
 * the spawn row's SLOT (its `seq`/position in issue order) is kept, but its
 * CONTENT is replaced by the subagent activity row, and the subagent row's own
 * synthetic entry is dropped so it is never emitted twice. Threads with no
 * `parentCallId` (older history) pass through unchanged — those still rely on
 * the reducer-side heuristic collapse.
 */
function resolveSubagentTimeline(
  timeline: readonly ToolTimelineEntry[]
): readonly ToolTimelineEntry[] {
  const byParentCallId = new Map<string, ToolTimelineEntry>();
  for (const entry of timeline) {
    if (entry.subagent?.parentCallId) byParentCallId.set(entry.subagent.parentCallId, entry);
  }
  if (byParentCallId.size === 0) return timeline;
  const substituted = new Set(byParentCallId.values());
  return timeline
    .filter(entry => !substituted.has(entry))
    .map(entry => {
      const subagentEntry = byParentCallId.get(entry.id);
      if (!subagentEntry) return entry;
      // Keep the SPAWN row's `id`/`seq` (its slot in issue order — what the
      // transcript's `toolCall` pointers and `unreferenced` sort key both key
      // off) but the SUBAGENT row's content, so a `spawn_subagent`/
      // `delegate_*` call and the delegation it started render as one part.
      return { ...subagentEntry, id: entry.id, seq: entry.seq };
    });
}

/** One item of a sub-agent's transcript, normalized to the `{kind:'tool', ...}` shape. */
function subagentTranscriptItems(activity: SubagentActivity): readonly SubagentTranscriptItem[] {
  if (activity.transcript && activity.transcript.length > 0) return activity.transcript;
  return activity.toolCalls.map(call => ({ kind: 'tool' as const, ...call }));
}

/** A sub-agent's child tool call as a plain (non-nested) `tool-call` part. */
function subagentChildToolPart(
  item: Extract<SubagentTranscriptItem, { kind: 'tool' }>
): ToolCallMessagePart {
  const running = isActiveTimelineStatus(item.status);
  const args = jsonObject(item.args);
  return {
    type: 'tool-call',
    toolCallId: item.callId,
    toolName: item.toolName,
    args,
    argsText: JSON.stringify(args, null, 2),
    ...(!running
      ? {
          result:
            item.status === 'error' || item.status === 'cancelled'
              ? {
                  status: item.status,
                  failure: item.failure,
                  ...(item.result !== undefined ? { value: item.result } : {}),
                }
              : (item.result ?? { status: item.status }),
        }
      : {}),
  };
}

/**
 * A sub-agent delegation's full run, as the nested `ThreadMessage[]` a
 * `task` part's `messages` field carries (assistant-ui's `TaskCard`/
 * `ReadonlyThreadProvider` convention — see `elements/task-card.aui.tsx`).
 *
 * One opening `user` message for the parent's delegation prompt (the
 * "instruction" row), then one `assistant` message replaying the child's own
 * thinking/text/tool-call sequence in the order it happened. Built with
 * `fromThreadMessageLike` — the same `ThreadMessageLike` shape this module's
 * own `toThreadMessageLike` produces for the top-level thread — rather than
 * hand-assembling a full `ThreadMessage`, which carries several
 * runtime-internal fields (branching, per-part provider metadata) that have
 * no source of truth on `SubagentActivity` and are not this adapter's to
 * invent.
 */
export function subagentMessages(activity: SubagentActivity): readonly AuiThreadMessage[] {
  const likes: ThreadMessageLike[] = [];
  if (activity.prompt?.trim()) {
    likes.push({ role: 'user', content: [{ type: 'text', text: activity.prompt }] });
  }
  const parts: ThreadAssistantMessagePart[] = [];
  for (const item of subagentTranscriptItems(activity)) {
    if (item.kind === 'thinking') {
      if (item.text.trim().length > 0) parts.push(reasoningPart(item.text, undefined, undefined));
      continue;
    }
    if (item.kind === 'text') {
      if (item.text.trim().length > 0) parts.push({ type: 'text', text: item.text });
      continue;
    }
    parts.push(subagentChildToolPart(item));
  }
  if (parts.length > 0) {
    const running = isActiveTimelineStatus(activity.status);
    likes.push({
      role: 'assistant',
      content: parts,
      status: running
        ? { type: 'running' }
        : activity.status === 'failed' || activity.status === 'error'
          ? { type: 'incomplete', reason: 'error' }
          : activity.status === 'cancelled'
            ? { type: 'incomplete', reason: 'cancelled' }
            : activity.status === 'incomplete'
              ? { type: 'incomplete', reason: 'other' }
              : { type: 'complete', reason: 'stop' },
    });
  }
  return likes.map((like, index) =>
    fromThreadMessageLike(like, `${activity.taskId}:${index}`, { type: 'complete', reason: 'stop' })
  );
}

function toolPart(entry: ToolTimelineEntry): ThreadAssistantMessagePart {
  const running = isActiveTimelineStatus(entry.status);
  const isSubagent = entry.name.startsWith('subagent:') || entry.subagent !== undefined;
  const args = isSubagent
    ? jsonObject({
        subagent_type: entry.subagent?.agentId ?? entry.name.replace(/^subagent:/, ''),
        description: entry.detail,
        ...(running ? { progress: entry.subagent } : {}),
      })
    : toolArgs(entry);

  // The spawn/delegate call's own real `tool_call_id`, when the core told us
  // which one started this delegation — see `resolveSubagentTimeline`. Using
  // it here (rather than this row's synthetic id) is what lets the part
  // render as ONE task card on the exact call the model made, instead of two
  // separate rows.
  const toolCallId = isSubagent ? (entry.subagent?.parentCallId ?? entry.id) : entry.id;
  const nestedMessages = isSubagent && entry.subagent ? subagentMessages(entry.subagent) : [];

  return {
    type: 'tool-call',
    toolCallId,
    toolName: isSubagent ? 'task' : entry.name,
    args,
    argsText: JSON.stringify(args, null, 2),
    ...(!isSubagent && toolArtifact(entry) ? { artifact: toolArtifact(entry) } : {}),
    ...(nestedMessages.length > 0 ? { messages: nestedMessages } : {}),
    ...(!running
      ? {
          result: isSubagent
            ? { status: entry.status, activity: entry.subagent }
            : toolResultPayload(entry),
        }
      : {}),
  };
}

/**
 * The decision set offered for a parked tool call.
 *
 * Each `id` is verbatim the `decision` literal `openhuman.approval_decide`
 * takes, so the runtime hands the adapter an option id it can forward to the
 * RPC without a translation table — see `useOpenHumanExternalStore`. The
 * `kind`s are what assistant-ui resolves to the boolean `approved` it reports
 * alongside the id, and what a renderer keys its default labels off.
 *
 * `reject-always` is deliberately absent: the core has no "never allow this
 * tool" decision, and offering one that silently degrades to a one-off deny
 * would be a lie about what the click did.
 */
export const APPROVAL_DECISION_OPTIONS: readonly ToolApprovalOption[] = [
  { id: 'approve_once', kind: 'allow-once' },
  { id: 'approve_always_for_tool', kind: 'allow-always' },
  { id: 'deny', kind: 'reject-once' },
];

/**
 * `toolCallId` prefix for the part synthesised when a parked approval cannot be
 * matched to a live timeline row. Namespaced so it can never collide with a
 * core-issued tool-call id (the duplicate-id invariant above is a hard one).
 */
const APPROVAL_PART_ID_PREFIX = '__openhuman_approval__:';

/**
 * The part-level `approval` field, projected from our `PendingApproval`.
 *
 * Shape mirrors assistant-ui's own `ToolCallMessagePart['approval']`
 * (`@assistant-ui/core`): `resolution` is the terminal non-decision state a
 * server-recorded TTL expiry or cancel sets (`approval_decided` socket event,
 * see `chatRuntimeSlice.ts`'s `resolvePendingApprovalForThread`); `approved`
 * follows it (`false`) so a renderer that only checks the boolean still shows
 * a resolved state rather than a live prompt.
 */
function approvalField(approval: PendingApproval): NonNullable<ToolCallMessagePart['approval']> {
  return {
    id: approval.requestId,
    options: APPROVAL_DECISION_OPTIONS,
    ...(approval.resolution ? { resolution: approval.resolution, approved: false as const } : {}),
  };
}

/** The part the parked call is asking about, when no timeline row carries it. */
function syntheticApprovalPart(approval: PendingApproval): ThreadAssistantMessagePart {
  // `command` is the redacted command/path/url the gate extracted for display;
  // rendering it as the call's args is what makes the prompt answerable.
  const args = approval.command ? { command: approval.command } : {};
  return {
    type: 'tool-call',
    toolCallId: approval.toolCallId ?? `${APPROVAL_PART_ID_PREFIX}${approval.requestId}`,
    toolName: approval.toolName,
    args: args as Record<string, never>,
    argsText: JSON.stringify(args, null, 2),
    approval: approvalField(approval),
  };
}

/**
 * Hang a parked approval off the tool part it is gating.
 *
 * `approval.toolCallId` (wire contract: `DomainEvent::ApprovalRequested.
 * tool_call_id`, additive) is preferred when present: it names the EXACT
 * part the gate is holding, so the match is an equality check rather than a
 * guess. A core that has not landed the C2 approvals workstream yet sends no
 * `tool_call_id`, and the older heuristic — the newest still-unsettled call
 * with the same tool name (a `result` means the call already ran and cannot
 * be the one parked) — remains the fallback for exactly that case, not a
 * second attempt after a failed exact match: once the wire names the part,
 * guessing at a different one would be worse than not finding it. When
 * nothing matches (the progress channel is bounded and can drop the
 * `tool_call` frame, and the gate can park before the frame lands at all) a
 * part is synthesised rather than dropped: a prompt in the wrong visual slot
 * is recoverable, a turn that parks with no prompt at all is the bug this
 * exists to close.
 */
function withApproval(
  parts: ThreadAssistantMessagePart[],
  approval: PendingApproval
): ThreadAssistantMessagePart[] {
  const index = approval.toolCallId
    ? parts.findIndex(part => part.type === 'tool-call' && part.toolCallId === approval.toolCallId)
    : parts.reduce(
        (best, part, at) =>
          part.type === 'tool-call' &&
          part.toolName === approval.toolName &&
          part.result === undefined
            ? at
            : best,
        -1
      );
  if (index < 0) return [...parts, syntheticApprovalPart(approval)];
  return parts.map((part, at) =>
    at === index ? { ...part, approval: approvalField(approval) } : part
  );
}

/**
 * A reasoning part, carrying the block's timing (epoch ms) under
 * `providerMetadata.openhuman` when it is known. `OpenHumanReasoningGroup`
 * reads it back (`reasoningTimingOf`) to show "Thinking… Ns" while the block
 * streams and "Thought for Ns" once it settles. Blocks recorded before timing
 * existed carry none and settle to a plain "Thought".
 */
export function reasoningPart(
  text: string,
  startedAt: number | undefined,
  endedAt: number | undefined
): ThreadAssistantMessagePart {
  if (startedAt === undefined && endedAt === undefined) return { type: 'reasoning', text };
  const timing: { startedAt?: number; endedAt?: number } = {};
  if (startedAt !== undefined) timing.startedAt = startedAt;
  if (endedAt !== undefined) timing.endedAt = endedAt;
  return { type: 'reasoning', text, providerMetadata: { openhuman: timing } };
}

/**
 * Project one assistant message into assistant-ui parts.
 *
 * The surface carries the turn as it happened, in the order it happened:
 * reasoning as a disclosure, what the agent said between tool rounds, every
 * tool row where it was issued, then the answer.
 *
 * ## One shape, live and settled
 *
 * The live tail and the settled message are the SAME projection of the same
 * transcript, and that is the property everything else here serves. A live
 * turn that projected differently from its settled self (text always last,
 * narration shown then wiped, reasoning unshifted to the front) re-shaped on
 * every event and again at completion — and assistant-ui keys text and
 * reasoning parts by their INDEX, so each reshaping remounted the markdown,
 * restarted its reveal from nothing, reset every disclosure and jumped the
 * scroll. So:
 *
 * - **Reasoning** (`kind: 'thinking'`) renders through the static reasoning
 *   panel: one "Thought for Ns" line once settled, titled steps while it
 *   streams.
 * - **Narration renders inline** where it was said, live and on reload. It is
 *   the agent explaining the call it is about to make; showing it and then
 *   wiping it was the flicker, never showing it on reload was the mismatch.
 * - **Parts are append-only while a turn streams.** New events add parts at
 *   the end; nothing is inserted before an existing part.
 * - **The answer takes the final narration's slot.** Live, the final round's
 *   text IS a narration item (`streamDeltaReceived` coalesces every content
 *   delta into one per round). Settled, the persisted `msg.content` is that
 *   same text, so narration after the turn's last tool call is replaced by the
 *   answer in place: same index, same key, no remount at completion. The core
 *   projection (`mapDisplayItems`) only emits interim narration, so a reloaded
 *   turn has no trailing narration and the answer lands in the same place.
 *
 * ## Ordering
 *
 * The transcript is the ordered record of the turn and is walked in array
 * order. Timeline rows the transcript never names — a legacy snapshot has no
 * transcript at all, and a live turn can mint a row before its pointer lands —
 * are merged in by their own `seq` rather than appended after everything else,
 * which is what previously let a row the agent issued FIRST render last.
 *
 * The two `seq` fields are NOT one ordering space: a live transcript item's
 * `seq` is its index in the transcript array while a timeline row's comes from
 * the per-thread `toolTimelineSeqByThread` counter. So the merge below compares
 * timeline `seq` to timeline `seq` only, never across the two.
 *
 * **Every tool part must have a distinct `toolCallId`.** assistant-ui keys them
 * as `toolCallId-${id}` and *throws* on a repeat ("Duplicate key … in
 * useResources"), which takes the whole thread render down rather than dropping
 * a row — so this is a hard invariant, not a tidiness rule, and it is enforced
 * here at the boundary as well as at each producer. `claim` guards every emit
 * below; the sources upstream (the live Redux slice and the derived transcript
 * mapper) also mint unique ids, but threads persisted before those fixes still
 * carry colliding ones.
 */
function assistantParts(
  answer: string,
  timeline: readonly ToolTimelineEntry[],
  transcript: readonly ProcessingTranscriptItem[],
  mode: 'live' | 'settled',
  citations: readonly ChatCitation[] = EMPTY_CITATIONS
): ThreadAssistantMessagePart[] {
  const resolvedTimeline = resolveSubagentTimeline(timeline);
  const parts: ThreadAssistantMessagePart[] = [];
  const timelineById = new Map(resolvedTimeline.map(entry => [entry.id, entry]));
  const emittedToolIds = new Set<string>();
  const claim = (entry: ToolTimelineEntry): boolean => {
    if (emittedToolIds.has(entry.id)) return false;
    emittedToolIds.add(entry.id);
    return true;
  };

  // Rows the transcript names. Two pointers can resolve to the same row (a
  // provider that emits tool calls without ids writes the empty string for all
  // of them), which is why this is a set of resolved row ids rather than a
  // count of pointers.
  const referenced = new Set<string>();
  let lastToolPointer = -1;
  for (const [index, item] of transcript.entries()) {
    if (item.kind !== 'toolCall') continue;
    const entry = timelineById.get(item.callId);
    if (entry) {
      referenced.add(entry.id);
      lastToolPointer = index;
    }
  }

  // Rows with no pointer, oldest first. These are merged into the walk below
  // rather than appended after it.
  const unreferenced = resolvedTimeline
    .filter(entry => !referenced.has(entry.id))
    .sort((a, b) => a.seq - b.seq);
  let nextUnreferenced = 0;
  /** Emit every unreferenced row the agent issued before `seq` (all of them at the end). */
  const drainBefore = (seq: number | null) => {
    while (nextUnreferenced < unreferenced.length) {
      const entry = unreferenced[nextUnreferenced];
      if (entry === undefined || (seq !== null && entry.seq >= seq)) break;
      nextUnreferenced += 1;
      if (claim(entry)) parts.push(toolPart(entry));
    }
  };

  // Settled: narration after the last tool call is the answer, spoken live;
  // the persisted answer replaces it in its slot. With no answer to show
  // (an empty or stopped reply) the narration stays — it is all there is.
  const answerText = answer.trim().length > 0 ? answer : '';
  const replaceTrailingNarration = mode === 'settled' && answerText.length > 0;
  let answerEmitted = false;

  for (const [index, item] of transcript.entries()) {
    if (item.kind === 'thinking') {
      if (item.text.trim().length > 0) {
        parts.push(
          reasoningPart(item.text, item.startedAt ?? undefined, item.endedAt ?? undefined)
        );
      }
      continue;
    }
    if (item.kind === 'narration') {
      if (item.text.trim().length === 0) continue;
      if (replaceTrailingNarration && index > lastToolPointer) {
        if (!answerEmitted) parts.push({ type: 'text', text: answerText });
        answerEmitted = true;
        continue;
      }
      // The answer is never narration, wherever a transcript puts it. A core
      // that predates the prompt-guided projection fix records a text-mode
      // turn's answer as an interim step (with the turn's calls after it); as
      // narration it would render the answer twice.
      if (mode === 'settled' && answerText.length > 0 && item.text.trim() === answerText.trim()) {
        continue;
      }
      parts.push({ type: 'text', text: item.text });
      continue;
    }
    if (item.kind !== 'toolCall') continue;
    const entry = timelineById.get(item.callId);
    if (!entry) continue;
    drainBefore(entry.seq);
    if (claim(entry)) parts.push(toolPart(entry));
  }
  drainBefore(null);

  // Settled with no trailing narration to stand in for (a reloaded turn, or a
  // legacy trail): the answer closes the turn. Live, the text is the
  // transcript's narration; `streamingTailMessage` handles the rare turn whose
  // transcript recorded none.
  if (mode === 'settled' && !answerEmitted && answerText.length > 0) {
    parts.push({ type: 'text', text: answerText });
  }
  // `extractAgentSources` is the one place a model-supplied URL is admitted
  // (http(s) only), so sources are derived through it rather than here.
  for (const source of extractAgentSources([...timeline])) {
    parts.push({
      type: 'source',
      sourceType: 'url',
      id: source.id,
      url: source.url,
      title: source.title,
    });
  }
  // Memory citations captured during retrieval for this turn
  // (`ChatDoneEvent.citations` / `ChatSegmentEvent.citations`), surfaced as
  // `document` source parts alongside the turn's `url` sources.
  for (const citation of citations) {
    parts.push({
      type: 'source',
      sourceType: 'document',
      id: `memory:${citation.id}`,
      title: citation.key,
      mediaType: 'application/vnd.openhuman.memory-citation',
    });
  }
  return parts;
}

function stringArray(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === 'string')
    : [];
}

function requestIdOf(message: ThreadMessage): string | undefined {
  const requestId = message.extraMetadata?.requestId;
  return typeof requestId === 'string' && requestId.length > 0 ? requestId : undefined;
}

/**
 * Memory citations `ChatRuntimeProvider` merged onto this message's
 * `extraMetadata.citations` (`chatDoneExtraMetadata` / the `onSegment`
 * handler in `ChatRuntimeProvider.tsx`). Narrowed rather than cast:
 * `extraMetadata` is untyped JSON from disk.
 */
function messageCitations(message: ThreadMessage): readonly ChatCitation[] {
  const value = message.extraMetadata?.citations;
  if (!Array.isArray(value)) return EMPTY_CITATIONS;
  return value.filter(
    (item): item is ChatCitation =>
      !!item &&
      typeof item === 'object' &&
      typeof (item as ChatCitation).id === 'string' &&
      typeof (item as ChatCitation).key === 'string'
  );
}

function isGenericToolName(name: string): boolean {
  return ['', 'tool', 'unknown', 'unknown_tool'].includes(name.trim().toLowerCase());
}

function recoverTimelineToolNames(
  timeline: readonly ToolTimelineEntry[],
  recoveredNames: readonly string[]
): readonly ToolTimelineEntry[] {
  if (recoveredNames.length === 0 || !timeline.some(entry => isGenericToolName(entry.name))) {
    return timeline;
  }
  // Advance only when a name is actually consumed. `recoveredNames` comes from
  // tool-call envelopes, so it is not positionally aligned with the whole
  // timeline: incrementing on every entry made `[read_file, tool, tool]` +
  // `[web_search, web_fetch]` mis-assign `web_fetch` to the first generic row
  // and leave the second one named `tool`.
  let recoveredIndex = 0;
  return timeline.map(entry => {
    if (!isGenericToolName(entry.name)) return entry;
    const recovered = recoveredNames[recoveredIndex];
    if (!recovered) return entry;
    recoveredIndex += 1;
    return { ...entry, name: recovered };
  });
}

function mergedAssistantText(messages: readonly ThreadMessage[]): string {
  const texts = messages
    .map(message => unwrapToolCallEnvelope(message.content ?? '').text)
    .filter(text => text.trim().length > 0)
    .filter((text, index, all) => all.indexOf(text) === index);
  if (texts.length < 2) return texts[0] ?? '';

  // Legacy web delivery persisted both paragraph-sized segments and the full
  // final response. Prefer the complete response when it contains every
  // segment; otherwise retain each distinct assistant emission in order.
  const longest = [...texts].sort((left, right) => right.length - left.length)[0] ?? '';
  if (texts.every(text => longest.includes(text.trim()))) return longest;
  return texts.join('\n\n');
}

const mergedRunCache = new WeakMap<
  ThreadMessage,
  { rows: readonly ThreadMessage[]; merged: ThreadMessage }
>();

function mergeAssistantRun(messages: readonly ThreadMessage[]): ThreadMessage {
  const firstRow = messages[0];
  const cached = firstRow && mergedRunCache.get(firstRow);
  if (
    cached &&
    cached.rows.length === messages.length &&
    cached.rows.every((row, index) => row === messages[index])
  )
    return cached.merged;
  const merged = mergeAssistantRunUncached(messages);
  if (firstRow && messages.length > 1) mergedRunCache.set(firstRow, { rows: messages, merged });
  return merged;
}

function mergeAssistantRunUncached(messages: readonly ThreadMessage[]): ThreadMessage {
  if (messages.length === 1) return messages[0];
  const first = messages[0];
  const last = messages[messages.length - 1];
  const extraMetadata = Object.assign({}, ...messages.map(message => message.extraMetadata));
  const requestId = messages.map(requestIdOf).find(Boolean);
  const toolNames = messages.flatMap(
    message => unwrapToolCallEnvelope(message.content ?? '').toolNames
  );
  if (requestId) extraMetadata.requestId = requestId;
  if (toolNames.length > 0) extraMetadata[RECOVERED_TOOL_NAMES_KEY] = toolNames;
  // Defect B: this one visible message is several persisted rows, and the
  // feedback adapter is only ever handed the last row's id (`...last` below).
  // Carry the whole set so a rating written against that id stays attributable
  // to what the user actually saw, rather than to the final fragment of it.
  extraMetadata[FEEDBACK_ROW_IDS_METADATA_KEY] = messages.map(message => message.id);
  // A merged run inherits a rating from ANY of its rows: the row the adapter
  // wrote to is the last one, but an earlier persist (or a re-merge with
  // different boundaries) can leave it elsewhere.
  const merged = messages.map(persistedFeedback).find(Boolean);
  if (merged) extraMetadata[FEEDBACK_METADATA_KEY] = merged;
  return {
    ...last,
    content: mergedAssistantText(messages),
    createdAt: first.createdAt,
    extraMetadata,
  };
}

/**
 * A row the core persisted as a complete, self-contained delivery — an
 * background sub-agent result, a worker-thread hand-off, or a workflow proposal. Core
 * writers stamp `extraMetadata.scope` on every such row; the legacy segmented
 * path never did. That positive marker is what tells an async delivery apart
 * from a paragraph of the answer next to it, since neither carries a request
 * id on the wire.
 */
function isStandaloneDelivery(message: ThreadMessage): boolean {
  return typeof message.extraMetadata?.scope === 'string';
}

/**
 * Collapse legacy paragraph/tool-envelope rows into one assistant turn.
 *
 * The old interactive-web delivery path persisted each segment as a separate
 * agent message. Consecutive assistant rows cannot cross a user turn; when
 * both rows carry request ids, a differing id is the explicit boundary, and a
 * scoped standalone delivery ({@link isStandaloneDelivery}) is always its own
 * turn — it neither joins the run before it nor seeds the run after it.
 */
function coalesceAssistantSegments(messages: readonly ThreadMessage[]): ThreadMessage[] {
  const out: ThreadMessage[] = [];
  let run: ThreadMessage[] = [];
  let runRequestId: string | undefined;

  const flush = () => {
    if (run.length > 0) out.push(mergeAssistantRun(run));
    run = [];
    runRequestId = undefined;
  };

  for (const message of messages) {
    if (message.sender !== 'agent' || message.extraMetadata?.hidden) {
      flush();
      out.push(message);
      continue;
    }
    if (isStandaloneDelivery(message)) {
      flush();
      run.push(message);
      flush();
      continue;
    }
    const requestId = requestIdOf(message);
    if (run.length > 0 && runRequestId && requestId && runRequestId !== requestId) flush();
    run.push(message);
    runRequestId ??= requestId;
  }
  flush();
  return out;
}

function mimeTypeFromDataUri(dataUri: string): string {
  return dataUri.match(/^data:([^;,]+)/i)?.[1] ?? 'application/octet-stream';
}

function userParts(msg: ThreadMessage): ThreadUserMessagePart[] {
  const references = parseAttachmentReferences(msg.content ?? '');
  const parsed = parseMessageImages(references.text);
  const metadata = msg.extraMetadata ?? {};
  const kinds = stringArray(metadata.attachmentKinds);
  const names = stringArray(metadata.attachmentNames);
  const posters = stringArray(metadata.attachmentPosters);
  const metadataUris = stringArray(metadata.attachmentDataUris);
  const dataUris = metadataUris.length > 0 ? metadataUris : parsed.dataUris;
  const parts: ThreadUserMessagePart[] = [];

  if (parsed.text.length > 0) parts.push({ type: 'text', text: parsed.text });

  if (references.attachments.length > 0) {
    // Workspace paths cannot be loaded as browser images. A file chip restores
    // the attachment's identity without storing preview bytes in the message log.
    for (const file of references.attachments) {
      parts.push({ type: 'file', filename: file.name, data: '', mimeType: file.mime });
    }
    for (const image of parsed.dataUris) parts.push({ type: 'image', image });
    return parts;
  }

  if (kinds.length === 0) {
    for (const [index, image] of dataUris.entries()) {
      parts.push({ type: 'image', image, filename: names[index] });
    }
    return parts;
  }

  for (const [index, kind] of kinds.entries()) {
    const filename = names[index];
    if (kind === 'image') {
      const image = dataUris[index];
      if (image) parts.push({ type: 'image', image, filename });
      continue;
    }
    if (kind === 'video') {
      const image = posters[index];
      if (image) parts.push({ type: 'image', image, filename });
      else parts.push({ type: 'file', filename, data: '', mimeType: 'video/mp4' });
      continue;
    }
    const data = dataUris[index] ?? '';
    parts.push({ type: 'file', filename, data, mimeType: mimeTypeFromDataUri(data) });
  }
  return parts;
}

export function toThreadMessageLike(
  msg: ThreadMessage,
  timeline: readonly ToolTimelineEntry[] = EMPTY_TIMELINE,
  transcript: readonly ProcessingTranscriptItem[] = EMPTY_TRANSCRIPT
): ThreadMessageLike {
  const cached = conversionCache.get(msg);
  if (cached?.timeline === timeline && cached.transcript === transcript) return cached.converted;

  const unwrapped = unwrapToolCallEnvelope(msg.content ?? '');
  const text = msg.sender === 'agent' ? unwrapped.text : (msg.content ?? '');
  const recoveredToolNames = [
    ...unwrapped.toolNames,
    ...stringArray(msg.extraMetadata?.[RECOVERED_TOOL_NAMES_KEY]),
  ];
  const effectiveTimeline = recoverTimelineToolNames(timeline, recoveredToolNames);
  const feedback = msg.sender === 'agent' ? persistedFeedback(msg) : undefined;
  // `chat_done.timing` (wire-contract.md), stamped onto `extraMetadata` by
  // `ChatRuntimeProvider`'s `chatDoneExtraMetadata`. `streamStartTime` is
  // required by assistant-ui's `MessageTiming` type but not read by the
  // vendored `MessageTiming` element (`message-timing.aui.tsx` reads only
  // `firstTokenTime`/`totalStreamTime`/`tokensPerSecond`/`totalChunks`), so
  // the message's own `createdAt` is a reasonable value for it. `totalChunks`
  // has no wire counterpart yet, hence `0` rather than an invented count.
  const timingWire =
    msg.sender === 'agent'
      ? (msg.extraMetadata?.[TIMING_METADATA_KEY] as
          | { first_token_ms?: number; first_tool_ms?: number; total_ms?: number }
          | undefined)
      : undefined;
  const timing = timingWire
    ? {
        streamStartTime: new Date(msg.createdAt).getTime(),
        firstTokenTime: timingWire.first_token_ms,
        totalStreamTime: timingWire.total_ms,
        totalChunks: 0,
        toolCallCount: effectiveTimeline.length,
      }
    : undefined;
  // Socket errors are persisted as assistant rows, but assistant-ui's error
  // status should render them through MessageError's ErrorState card rather
  // than Markdown. The guardrail has its own structured notice card.
  const chatError =
    msg.sender === 'agent'
      ? (msg.extraMetadata?.[CHAT_ERROR_METADATA_KEY] as { errorType?: string } | undefined)
      : undefined;
  const isGuardrailError = chatError?.errorType === 'guardrail';
  const isChatError = chatError !== undefined && !isGuardrailError;
  // Older persisted errors may contain the retired custom navigation tag.
  // Keep the diagnostic text and provider detail, without exposing raw markup
  // inside the plain-text error card.
  const errorDetail = isChatError
    ? parseBubbleSegments(text)
        .filter(segment => segment.kind === 'text')
        .map(segment => segment.text)
        .join('')
        .replace(/\n{3,}/g, '\n\n')
        .trim()
    : '';

  const converted: ThreadMessageLike = {
    id: msg.id,
    role: msg.sender === 'agent' ? 'assistant' : 'user',
    content: chatError
      ? []
      : msg.sender === 'agent'
        ? assistantParts(text, effectiveTimeline, transcript, 'settled', messageCitations(msg))
        : userParts(msg),
    createdAt: new Date(msg.createdAt),
    ...(isChatError
      ? {
          status: {
            type: 'incomplete' as const,
            reason: 'error' as const,
            ...(errorDetail ? { error: errorDetail } : {}),
          },
        }
      : msg.sender === 'agent' && msg.extraMetadata?.stopped === true
        ? { status: { type: 'incomplete' as const, reason: 'cancelled' as const } }
        : {}),
    metadata: {
      // Defect A (#6459-adjacent, but its own bug): the runtime writes
      // `submittedFeedback` onto its OWN repository copy when a thumb is
      // pressed, and we supply `messages` rather than `messageRepository` — so
      // the runtime rebuilds from this converter's output on every store update
      // (`external-store-thread-runtime-core.js`) and that write is discarded.
      // Re-emitting it from the persisted value is what makes a pressed thumb
      // survive the next turn, a thread switch and a reload. Without this the
      // control silently un-presses, which is worse than having no control.
      ...(feedback ? { submittedFeedback: { type: feedback } } : {}),
      ...(timing ? { timing } : {}),
      custom: { extraMetadata: msg.extraMetadata ?? {}, sourceType: msg.type },
    },
  };

  conversionCache.set(msg, { timeline, transcript, converted });
  return converted;
}

/**
 * The live tail as a running assistant message.
 *
 * The tail is deliberately NOT part of `thread.messagesByThreadId` — Redux keeps
 * the settled transcript and the in-flight preview in separate slices, which is
 * exactly what keeps settled message identities stable while tokens land. Here
 * that separation is re-joined for the runtime's benefit: one fresh object per
 * token, and only that one object is ever re-converted.
 */
export function streamingTailMessage(
  streaming: StreamingAssistantState | null,
  timeline: readonly ToolTimelineEntry[] = EMPTY_TIMELINE,
  transcript: readonly ProcessingTranscriptItem[] = EMPTY_TRANSCRIPT,
  approval: PendingApproval | null = null,
  requestId: string | undefined = streaming?.requestId
): ThreadMessageLike | null {
  if (!approval && !streaming && timeline.length === 0 && transcript.length === 0) return null;
  let parts = assistantParts('', timeline, transcript, 'live');
  // The streaming buffers, for a turn whose transcript has not recorded them
  // (a snapshot-hydrated turn mid-answer): normally `streamDeltaReceived`
  // writes every thinking and content delta into the transcript as well, and
  // `assistantParts` above already emits them in place — these would double
  // them. Reasoning before text: what the agent thought before it answered.
  //
  // Appended, never unshifted: the tail's parts are append-only (see
  // `assistantParts`), and a part inserted at the front shifts the index — and
  // so the key — of every part after it, remounting the answer mid-stream. A
  // turn that has so far produced only thinking still mints a tail, which is
  // the point: the block is the in-flight signal, alongside `RunningStatus`.
  if (streaming?.thinking.trim() && !transcript.some(item => item.kind === 'thinking')) {
    parts.push(
      reasoningPart(streaming.thinking, streaming.thinkingStartedAt, streaming.thinkingEndedAt)
    );
  }
  if (streaming?.content.trim() && !transcript.some(item => item.kind === 'narration')) {
    parts.push({ type: 'text', text: streaming.content });
  }
  if (approval) parts = withApproval(parts, approval);
  if (parts.length === 0) return null;
  // A sub-agent parked on `ask_user_clarification` is, like a parked
  // ApprovalGate request, a turn stopped on the user rather than a running
  // one. assistant-ui derives a tool-call part's own status from its
  // ENCLOSING message when the part has no `result` (`toMessagePartStatus`),
  // so this is the one place that can give the task card its `requires-action`
  // state — the part itself has no status field of its own.
  const hasAwaitingSubagent = timeline.some(entry => entry.subagent?.status === 'awaiting_user');
  return {
    id: approval?.detached
      ? `${STREAMING_TAIL_ID}:approval:${approval.requestId}`
      : streamingMessageId(requestId),
    role: 'assistant',
    content: parts,
    status:
      approval || hasAwaitingSubagent
        ? { type: 'requires-action', reason: 'interrupt' }
        : { type: 'running' },
    metadata: { custom: { requestId, streaming: true } },
  };
}

const settledStatusCache = new WeakMap<
  readonly ToolTimelineEntry[],
  { settled: readonly ToolTimelineEntry[]; merged: readonly ToolTimelineEntry[] }
>();

/**
 * A frozen live trail, with each still-running row settled from the core
 * projection's row of the same id.
 *
 * `chat_done` does not invent a status for a row that has no result yet; the
 * core projection settles it (to its real status, or `cancelled`). The frozen
 * trail keeps the live row ids — which is what keeps every card mounted — so
 * only status, result and failure are taken over, never the row. Sub-agent
 * rows carry different ids on the two sides and are left to their own events.
 * Returns the frozen array itself when nothing changes, so the conversion
 * cache keeps hitting.
 */
function withSettledStatuses(
  frozen: readonly ToolTimelineEntry[],
  settled: readonly ToolTimelineEntry[] | undefined
): readonly ToolTimelineEntry[] {
  if (!settled || !frozen.some(entry => isActiveTimelineStatus(entry.status))) return frozen;
  const cached = settledStatusCache.get(frozen);
  if (cached?.settled === settled) return cached.merged;
  const byId = new Map(settled.map(entry => [entry.id, entry]));
  let changed = false;
  const merged = frozen.map(entry => {
    if (!isActiveTimelineStatus(entry.status)) return entry;
    const final = byId.get(entry.id);
    if (!final || isActiveTimelineStatus(final.status)) return entry;
    changed = true;
    return {
      ...entry,
      status: final.status,
      result: final.result ?? entry.result,
      failure: final.failure ?? entry.failure,
    };
  });
  const result = changed ? merged : frozen;
  settledStatusCache.set(frozen, { settled, merged: result });
  return result;
}

export type AssistantUiProjection = {
  /** Whether the synthetic live tail has an active core turn driving it. */
  isRunning?: boolean;
  /**
   * The thread's parked ApprovalGate request, if any. Present means the turn is
   * blocked on the user, and the tail is minted even when nothing else would
   * mint one — a parked gate with no prompt is the failure mode this closes.
   */
  pendingApproval?: PendingApproval | null;
  liveTimeline?: readonly ToolTimelineEntry[];
  liveTranscript?: readonly ProcessingTranscriptItem[];
  /**
   * The request whose rows `liveTimeline` holds, when known
   * (`chatRuntime.toolTimelineRequestByThread`). A settled message never
   * borrows another request's live rows as its trail.
   */
  liveTimelineRequestId?: string;
  turnTimelines?: Readonly<Record<string, readonly ToolTimelineEntry[]>>;
  turnTranscripts?: Readonly<Record<string, readonly ProcessingTranscriptItem[]>>;
  /**
   * Trails of turns that settled while this thread was open, frozen at
   * settlement (`chatRuntime.settledTurnsByThread`). They win over the core
   * projection for their request, so a turn keeps the exact parts it streamed
   * with — see `ChatRuntimeState.settledTurnsByThread`.
   */
  settledTurns?: Readonly<
    Record<
      string,
      { timeline: readonly ToolTimelineEntry[]; transcript: readonly ProcessingTranscriptItem[] }
    >
  >;
  /** `request_id` of the turn the live tail stands for, when known. */
  liveRequestId?: string;
};

/**
 * The full thread as assistant-ui sees it: settled transcript, then the live
 * tail when one is in flight.
 *
 * Hidden messages are filtered the same way the transcript filters them, so the
 * runtime's view of the thread and the rendered view cannot disagree about what
 * the conversation contains.
 */
export function buildRuntimeMessages(
  messages: readonly ThreadMessage[],
  streaming: StreamingAssistantState | null,
  projection: AssistantUiProjection = {}
): ThreadMessageLike[] {
  // A parked approval outranks the lifecycle: `chat_done` has not fired (the
  // turn is stopped, not finished), but a snapshot race that reports the turn
  // settled must not swallow the only surface that can unblock it.
  const pendingApproval = projection.pendingApproval ?? null;
  const coalescedMessages = coalesceAssistantSegments(messages);
  const out: ThreadMessageLike[] = [];
  const claimedRequestIds = new Set(
    coalescedMessages.flatMap(message =>
      message.sender === 'agent' && typeof message.extraMetadata?.requestId === 'string'
        ? [message.extraMetadata.requestId]
        : []
    )
  );
  const projectedRequestIds = [
    ...new Set([
      ...Object.keys(projection.turnTimelines ?? {}),
      ...Object.keys(projection.turnTranscripts ?? {}),
    ]),
  ].filter(requestId => !claimedRequestIds.has(requestId));
  // Async acknowledgements/background deliveries can be persisted without
  // message-level request metadata. The transcript maps are chronological and
  // request-keyed, so unclaimed trails can be paired with unanchored agent
  // messages in order — but only when the two sets are the same size. With a
  // surplus of unanchored messages, positional pairing hands a later turn's
  // tools to an earlier trail-less answer and leaves the real answer bare;
  // rendering those trails nowhere is the lesser wrong.
  const unanchoredAgentCount = coalescedMessages.filter(
    message =>
      message.sender === 'agent' &&
      !message.extraMetadata?.hidden &&
      typeof message.extraMetadata?.requestId !== 'string'
  ).length;
  const pairOrphanTrails =
    projectedRequestIds.length > 0 && projectedRequestIds.length === unanchoredAgentCount;
  let orphanRequestCursor = 0;
  const lastVisibleAgentId = [...coalescedMessages]
    .reverse()
    .find(message => message.sender === 'agent' && !message.extraMetadata?.hidden)?.id;
  // A detached approval (an async sub-agent's) can still be parked after the
  // turn that surfaced it settled. That turn's retained trail is already shown
  // by its settled message, so the tail then carries only the approval —
  // re-rendering the trail beside it would duplicate every tool card.
  const detachedAfterTurn = projection.isRunning === false && pendingApproval?.detached === true;
  const tail =
    projection.isRunning === false && !pendingApproval
      ? null
      : streamingTailMessage(
          detachedAfterTurn ? null : streaming,
          detachedAfterTurn ? EMPTY_TIMELINE : (projection.liveTimeline ?? EMPTY_TIMELINE),
          detachedAfterTurn ? EMPTY_TRANSCRIPT : (projection.liveTranscript ?? EMPTY_TRANSCRIPT),
          pendingApproval,
          detachedAfterTurn ? undefined : (projection.liveRequestId ?? streaming?.requestId)
        );
  // While the tail stands for the live turn, that turn's own persisted rows
  // (the reply appended before `turnSettled`, or segments delivered mid-turn)
  // are not rendered beside it. Rendering both put the reply at the tail's
  // index and pushed the tail — with every tool card — one slot down, where
  // assistant-ui (which keys messages by index) remounted it. `turnSettled`
  // ends the tail and reveals the row in one store update, at the same index.
  const hiddenLiveRequestId = tail ? projection.liveRequestId : undefined;
  for (const msg of coalescedMessages) {
    if (msg.extraMetadata?.hidden) continue;
    const requestId =
      msg.sender === 'agent' && typeof msg.extraMetadata?.requestId === 'string'
        ? msg.extraMetadata.requestId
        : undefined;
    if (hiddenLiveRequestId !== undefined && requestId === hiddenLiveRequestId) continue;
    const frozen = requestId ? projection.settledTurns?.[requestId] : undefined;
    if (frozen) {
      const settledRows = requestId ? projection.turnTimelines?.[requestId] : undefined;
      out.push(
        toThreadMessageLike(
          msg,
          withSettledStatuses(frozen.timeline, settledRows),
          frozen.transcript
        )
      );
      continue;
    }
    const effectiveRequestId =
      requestId ??
      (msg.sender === 'agent' && pairOrphanTrails
        ? projectedRequestIds[orphanRequestCursor++]
        : undefined);
    const persistedTimeline = effectiveRequestId
      ? projection.turnTimelines?.[effectiveRequestId]
      : undefined;
    const persistedTranscript = effectiveRequestId
      ? projection.turnTranscripts?.[effectiveRequestId]
      : undefined;
    // `chat_done` clears the active lifecycle before the completed snapshot is
    // indexed into the request maps. Keep the just-settled tools/reasoning on
    // the final assistant message during that handoff; never mint a running
    // synthetic tail for them.
    // Not while a gate is parked: the tail below is minted unconditionally in
    // that case and would emit the same rows a second time, and a repeated
    // `toolCallId` throws inside assistant-ui rather than dropping a row.
    // Nor when those rows are provably another turn's: a reply that settled
    // with no `inference_start` of its own (a background delivery) otherwise
    // showed the previous turn's tool cards a second time under itself.
    const liveRowsAreForeign =
      projection.liveTimelineRequestId !== undefined &&
      requestId !== undefined &&
      projection.liveTimelineRequestId !== requestId;
    const useSettledLiveFallback =
      projection.isRunning === false &&
      // A detached approval's tail carries none of these rows (see
      // `detachedAfterTurn`), so the settled message keeps them.
      (!pendingApproval || detachedAfterTurn) &&
      msg.id === lastVisibleAgentId &&
      !persistedTimeline &&
      !persistedTranscript &&
      !liveRowsAreForeign;
    out.push(
      toThreadMessageLike(
        msg,
        persistedTimeline ??
          (useSettledLiveFallback ? (projection.liveTimeline ?? EMPTY_TIMELINE) : EMPTY_TIMELINE),
        persistedTranscript ??
          (useSettledLiveFallback
            ? (projection.liveTranscript ?? EMPTY_TRANSCRIPT)
            : EMPTY_TRANSCRIPT)
      )
    );
  }
  if (tail) out.push(tail);
  return out;
}
