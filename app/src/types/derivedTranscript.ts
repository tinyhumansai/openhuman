/**
 * Wire shape of the transcript-derived view RPC
 * (`openhuman.threads_transcript_get`, Phase B — see
 * `src/openhuman/threads/transcript_view/types.rs`).
 *
 * The Rust core projects the append-only `session_raw/*.jsonl` source of truth
 * into typed **display items** in the frontend's chat vocabulary. Phase C maps
 * these onto the existing settled-turn renderers (`PastTurnInsights` /
 * `ProcessingTranscriptView` / `ToolTimelineBlock` / `SubagentActivityBlock`)
 * via `features/conversations/derived/mapDisplayItems.ts`.
 *
 * Serde is camelCase on the wire, so every field here is camelCase and mirrors
 * the Rust `DisplayItem` / `TranscriptPage` serialization exactly.
 */

/**
 * Terminal state of a projected tool call. Mirrors the Rust `ToolCallStatus`
 * (snake_case on the wire) and the live timeline's `ToolTimelineStatus`
 * vocabulary so the settled projection and the live stream render identically.
 */
export type DerivedToolCallStatus = 'running' | 'success' | 'error';

/**
 * One item in a projected transcript, in the frontend's display vocabulary.
 * Discriminated by `kind` (camelCase discriminator from the Rust
 * `#[serde(tag = "kind")]` enum).
 */
export type DerivedDisplayItem =
  | DerivedUserMessage
  | DerivedAssistantMessage
  | DerivedReasoning
  | DerivedToolCall
  | DerivedSubagent
  | DerivedTurnBoundary
  | DerivedInterruptedPartial
  | DerivedCompaction;

/**
 * A user prompt. `content` is the raw persisted content (may carry an injected
 * `Current Date & Time:` scaffolding line); `displayContent` is the sanitized
 * version to show, present only when it differs from raw — prefer it.
 */
export interface DerivedUserMessage {
  kind: 'userMessage';
  content: string;
  displayContent?: string;
  requestId?: string;
}

/**
 * An assistant answer. `interim: true` marks a non-terminal tool-calling step
 * within a multi-iteration turn (narration between tool calls), **not** the
 * final answer bubble — the final (non-interim) answer stays rendered from the
 * thread message list.
 */
export interface DerivedAssistantMessage {
  kind: 'assistantMessage';
  content: string;
  interim?: boolean;
  requestId?: string;
  model?: string;
  iteration?: number;
}

/** The model's reasoning/thinking that preceded an assistant message. */
export interface DerivedReasoning {
  kind: 'reasoning';
  text: string;
}

/**
 * Failure payload attached to an errored tool call. Minimal on the wire (the
 * persisted transcript only records the failure plus an optional short reason);
 * the mapper expands it into the richer `ToolFailureExplanation` shape the
 * `ToolFailureLines` renderer consumes.
 */
export interface DerivedToolFailure {
  /** Short, single-line reason for the failure, when the writer captured one. */
  detail?: string;
}

/** A tool invocation with its paired result, when available. */
export interface DerivedToolCall {
  kind: 'toolCall';
  callId: string;
  name: string;
  args?: unknown;
  result?: string;
  status: DerivedToolCallStatus;
  /** Present only when `status` is `'error'`. */
  failure?: DerivedToolFailure;
}

/**
 * A delegated sub-agent run, with its own nested projected items. `requestId`
 * anchors the whole trail to the parent turn that spawned it (derived core-side
 * from the sub-agent's spawn timestamp vs. the parent turns' timestamp ranges);
 * absent for legacy/CLI transcripts with no `requestId`.
 */
export interface DerivedSubagent {
  kind: 'subagent';
  id: string;
  requestId?: string;
  items: DerivedDisplayItem[];
}

/** A turn boundary — emitted when the `requestId` changes between lines. */
export interface DerivedTurnBoundary {
  kind: 'turnBoundary';
  requestId: string;
}

/** A partial assistant answer captured when a turn was interrupted. */
export interface DerivedInterruptedPartial {
  kind: 'interruptedPartial';
  text: string;
  thinking?: string;
}

/**
 * A context-compaction marker: the reduced set replaced everything before it.
 * Carried for completeness; the settled-turn renderers have no compaction
 * concept, so the Phase C mapper currently drops it.
 */
export interface DerivedCompaction {
  kind: 'compaction';
  replacedCount: number;
  keptCount: number;
  ts?: string;
  requestId?: string;
}

/**
 * One newest-first page of a thread's projected transcript. `hasTranscript` is
 * `false` when the thread has no persisted transcript yet (legacy thread /
 * brand-new) — the caller then falls back to the turn_state hydration path.
 */
export interface DerivedTranscriptPage {
  threadId: string;
  /** Display items for this page, **newest-first**. */
  items: DerivedDisplayItem[];
  /** Total top-level items available for the thread. */
  total: number;
  /** Opaque cursor to pass back for the next (older) page; absent at the end. */
  nextCursor?: string;
  /** `true` when more (older) items remain beyond this page. */
  hasMore: boolean;
  /** `false` when the thread has no persisted transcript yet (empty page). */
  hasTranscript: boolean;
}

/** Optional pagination controls for {@link DerivedTranscriptPage} fetches. */
export interface DerivedTranscriptGetOptions {
  /** Opaque token from a prior page's `nextCursor`. */
  cursor?: string;
  /** Page size; core default 50, clamped to 500. */
  limit?: number;
}
