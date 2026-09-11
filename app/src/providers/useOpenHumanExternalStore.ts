import type { AppendMessage, RespondToToolApprovalOptions } from '@assistant-ui/react';
import { useCallback, useEffect, useMemo, useState } from 'react';

import { mapDisplayItems } from '../features/conversations/derived/mapDisplayItems';
import { type ApprovalDecision, decideApproval } from '../services/api/approvalApi';
import { threadApi } from '../services/api/threadApi';
import {
  clearPendingApprovalForThread,
  type InferenceStatus,
  isActiveTimelineStatus,
  type ToolTimelineEntry,
} from '../store/chatRuntimeSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import type { DerivedDisplayItem } from '../types/derivedTranscript';
import type { ThreadMessage } from '../types/thread';
import { buildRuntimeMessages } from './assistantUiMessages';
import { getChatSurface } from './chatSurfaceHandlers';

const EMPTY_MESSAGES: ThreadMessage[] = [];
const EMPTY_TIMELINE: never[] = [];
const EMPTY_TRANSCRIPT: never[] = [];
const EMPTY_TURN_MAP = {};
/** Items per derived-transcript RPC page; the core caps a page at this size. */
const DERIVED_TRANSCRIPT_PAGE_LIMIT = 500;
/**
 * Upper bound on pages walked for one thread (10k items). A thread longer than
 * this is truncated at its oldest end rather than fetched without limit; the
 * turn-bounded RPC contract that removes the ceiling altogether is tracked with
 * the transcript RPC, not here.
 */
const DERIVED_TRANSCRIPT_MAX_PAGES = 20;

/**
 * Everything the assistant-ui surface needs about the *currently running* turn
 * that is not a message, a tool part or a stream delta.
 *
 * assistant-ui derives its own running state from `thread.isRunning` alone, so
 * without this channel a long turn is a bare spinner: no phase, no round
 * counter, no active tool. The socket handlers already maintain all three in
 * `chatRuntime.inferenceStatusByThread` (`onInferenceStart`, `onIterationStart`,
 * `onToolCall`); this projects that slice onto the runtime the surface reads.
 *
 * It travels on the adapter's `extras` channel rather than being read from
 * Redux by the renderer so it stays scoped to *this runtime's* thread — the
 * Workflow Copilot mounts a second runtime on a thread that is deliberately not
 * `selectedThreadId`, and a renderer-side Redux read would paint the home
 * chat's progress inside it.
 */
export type OpenHumanThreadExtras = {
  /** Live phase/round/active-tool for the running turn, or `null` when idle. */
  inferenceStatus: InferenceStatus | null;
  /** Newest running non-subagent row, used to title the `tool_use` phase. */
  activeToolEntry?: ToolTimelineEntry | undefined;
  /** Running subagent row, used to title the `subagent` phase. */
  activeSubagentEntry?: ToolTimelineEntry | undefined;
};

const EMPTY_EXTRAS: OpenHumanThreadExtras = { inferenceStatus: null };

/**
 * Narrow assistant-ui's untyped `thread.extras` back to our own shape.
 *
 * `extras` is `unknown` by contract, and a surface can be mounted on a runtime
 * that is not ours (or on none at all), so this returns `null` rather than
 * asserting.
 */
export function readOpenHumanThreadExtras(extras: unknown): OpenHumanThreadExtras | null {
  if (typeof extras !== 'object' || extras === null) return null;
  if (!('inferenceStatus' in extras)) return null;
  return extras as OpenHumanThreadExtras;
}

type CoreTranscriptProjection = {
  threadId: string | null;
  timelines: ReturnType<typeof mapDisplayItems>['timelines'];
  transcripts: ReturnType<typeof mapDisplayItems>['transcripts'];
};

const EMPTY_CORE_TRANSCRIPT: CoreTranscriptProjection = {
  threadId: null,
  timelines: EMPTY_TURN_MAP,
  transcripts: EMPTY_TURN_MAP,
};

/**
 * Read settled process history straight from the core's transcript projection.
 * The Rust side owns a bounded, mtime-keyed LRU, so this hook deliberately does
 * not establish a second Redux transcript store or duplicate cache policy.
 */
export function useCoreTranscriptProjection(
  threadId: string | null,
  revision: string,
  liveRequestId: string | undefined
): CoreTranscriptProjection {
  const [projection, setProjection] = useState<CoreTranscriptProjection>(EMPTY_CORE_TRANSCRIPT);

  useEffect(() => {
    if (!threadId) {
      setProjection(EMPTY_CORE_TRANSCRIPT);
      return;
    }
    // Defensive for narrow test/embedder shims that expose only a subset of
    // threadApi. Production builds always provide this method.
    if (typeof threadApi.getDerivedTranscript !== 'function') {
      setProjection({ threadId, timelines: EMPTY_TURN_MAP, transcripts: EMPTY_TURN_MAP });
      return;
    }
    let cancelled = false;
    const skipRequestIds = liveRequestId ? new Set([liveRequestId]) : undefined;
    const project = (items: DerivedDisplayItem[]) => {
      const mapped = mapDisplayItems(items, { skipRequestIds });
      setProjection({ threadId, timelines: mapped.timelines, transcripts: mapped.transcripts });
    };
    void (async () => {
      try {
        const first = await threadApi.getDerivedTranscript(threadId, {
          limit: DERIVED_TRANSCRIPT_PAGE_LIMIT,
        });
        if (cancelled) return;
        if (!first.hasTranscript) {
          setProjection({ threadId, timelines: EMPTY_TURN_MAP, transcripts: EMPTY_TURN_MAP });
          return;
        }
        // Paint the newest page immediately, then walk the older pages and
        // re-project once with the whole history. A single page silently
        // dropped everything older than 500 items on a long thread, and a
        // page that begins mid-turn hides that turn's leading tool calls until
        // its boundary is in view — both only resolve with the full list.
        let items = first.items;
        project(items);
        let cursor = first.hasMore ? first.nextCursor : undefined;
        let pages = 1;
        while (cursor && pages < DERIVED_TRANSCRIPT_MAX_PAGES) {
          const page = await threadApi.getDerivedTranscript(threadId, {
            limit: DERIVED_TRANSCRIPT_PAGE_LIMIT,
            cursor,
          });
          if (cancelled) return;
          // Pages are newest-first and each next page is older, so appending
          // keeps the newest-first order `mapDisplayItems` expects.
          items = [...items, ...page.items];
          pages += 1;
          cursor = page.hasMore ? page.nextCursor : undefined;
        }
        if (pages > 1) project(items);
      } catch {
        // A missing/older core has no settled process trail; message text and
        // the live socket projection remain usable. Navigation must not fail.
        if (!cancelled) {
          setProjection({ threadId, timelines: EMPTY_TURN_MAP, transcripts: EMPTY_TURN_MAP });
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [liveRequestId, revision, threadId]);

  return projection.threadId === threadId ? projection : EMPTY_CORE_TRANSCRIPT;
}

/** Flatten an assistant-ui append payload down to the plain text our core takes. */
function appendMessageText(message: AppendMessage): string {
  return message.content
    .map(part => (part.type === 'text' ? part.text : ''))
    .join('')
    .trim();
}

/**
 * Build the `ExternalStoreAdapter` that backs `useExternalStoreRuntime`.
 *
 * Settled messages and live deltas remain in their existing UI stores, while
 * reasoning/tool/sub-agent history comes directly from the core transcript
 * projection. Redux is not a second transcript database.
 */
export function useOpenHumanExternalStore(threadId: string | null) {
  const dispatch = useAppDispatch();
  const messages = useAppSelector(state =>
    threadId ? (state.thread.messagesByThreadId[threadId] ?? EMPTY_MESSAGES) : EMPTY_MESSAGES
  );
  const streaming = useAppSelector(state =>
    threadId ? (state.chatRuntime.streamingAssistantByThread?.[threadId] ?? null) : null
  );
  const lifecycle = useAppSelector(state =>
    threadId ? (state.chatRuntime.inferenceTurnLifecycleByThread?.[threadId] ?? null) : null
  );
  const isLoading = useAppSelector(state => Boolean(threadId && state.thread.isLoadingMessages));
  const liveTimeline = useAppSelector(state =>
    threadId
      ? (state.chatRuntime.toolTimelineByThread?.[threadId] ?? EMPTY_TIMELINE)
      : EMPTY_TIMELINE
  );
  const liveTranscript = useAppSelector(state =>
    threadId
      ? (state.chatRuntime.processingByThread?.[threadId] ?? EMPTY_TRANSCRIPT)
      : EMPTY_TRANSCRIPT
  );
  // Progress status for the running turn. Cleared by the socket layer on turn
  // end/error/cancel, so its presence is itself the "still working" signal.
  const inferenceStatus = useAppSelector(state =>
    threadId ? (state.chatRuntime.inferenceStatusByThread?.[threadId] ?? null) : null
  );
  // The thread's parked ApprovalGate request. Selected here rather than read by
  // a card somewhere else on the page: the decision belongs on the tool call it
  // gates, and only this adapter can put it there.
  const pendingApproval = useAppSelector(state =>
    threadId ? (state.chatRuntime.pendingApprovalByThread?.[threadId] ?? null) : null
  );
  const settledRevision = `${messages.at(-1)?.id ?? ''}:${messages.at(-1)?.content?.length ?? 0}:${lifecycle ?? ''}`;
  const coreTranscript = useCoreTranscriptProjection(
    threadId,
    settledRevision,
    streaming?.requestId
  );

  // `started` and `streaming` are both in-flight. A completed turn can retain
  // its tool/reasoning arrays while the persisted projection catches up; those
  // arrays must not mint a forever-running assistant-ui tail.
  const isRunning = lifecycle === 'started' || lifecycle === 'streaming';

  // Recomputed only when the settled transcript or the live tail changes.
  // Settled messages are converted through an identity-keyed cache, so a token
  // landing on the tail re-converts exactly one message, never the transcript.
  const runtimeMessages = useMemo(
    () =>
      buildRuntimeMessages(messages, streaming, {
        isRunning,
        liveTimeline,
        liveTranscript,
        pendingApproval,
        turnTimelines: coreTranscript.timelines,
        turnTranscripts: coreTranscript.transcripts,
      }),
    [messages, streaming, isRunning, liveTimeline, liveTranscript, pendingApproval, coreTranscript]
  );

  // The status line titles its `tool_use` / `subagent` phases from the matching
  // running timeline row (the same rows the surface renders as tool parts), so
  // "Running command: npm test..." reads like the row rather than the raw tool
  // id. Resolved here so the renderer stays a pure projection of `extras`.
  const extras = useMemo<OpenHumanThreadExtras>(() => {
    if (!inferenceStatus) return EMPTY_EXTRAS;
    return {
      inferenceStatus,
      // `isActiveTimelineStatus`, not `status === 'running'`: a delegated child
      // parked on `ask_user_clarification` carries `awaiting_user` on the row's
      // top-level status, and dropping the match there loses the sub-agent's
      // identity at the one moment the user is the thing being waited on.
      activeToolEntry: [...liveTimeline]
        .reverse()
        .find(entry => isActiveTimelineStatus(entry.status) && !entry.name.startsWith('subagent:')),
      activeSubagentEntry: liveTimeline.find(
        entry => isActiveTimelineStatus(entry.status) && entry.name.startsWith('subagent:')
      ),
    };
  }, [inferenceStatus, liveTimeline]);

  const onNew = useCallback(
    async (message: AppendMessage) => {
      const surface = getChatSurface(threadId);
      // Fail loudly. A silent no-op here would look like a dropped message.
      if (!surface) {
        throw new Error(`No chat surface registered for thread ${threadId ?? '(none)'}`);
      }
      const text = appendMessageText(message);
      if (text.length === 0) return;
      await surface.send(text);
    },
    [threadId]
  );

  const onCancel = useCallback(async () => {
    await getChatSurface(threadId)?.cancel?.();
  }, [threadId]);

  /**
   * Record the user's decision on the parked tool call.
   *
   * `optionId` is the core's own `decision` literal (see
   * `APPROVAL_DECISION_OPTIONS`), so it forwards unchanged; the boolean
   * `approved` is only the fallback for a renderer that answered with a plain
   * allow/deny rather than picking one of the declared options.
   *
   * Supplying this at all is load-bearing, not optional: without it the runtime
   * *throws* `Runtime does not support tool approvals.` the moment a decision
   * button is pressed, rather than no-opping.
   */
  const onRespondToToolApproval = useCallback(
    async ({ approvalId, approved, optionId }: RespondToToolApprovalOptions) => {
      const decision = (optionId ?? (approved ? 'approve_once' : 'deny')) as ApprovalDecision;
      await decideApproval(approvalId, decision);
      // Resolve optimistically, exactly as `ApprovalRequestCard` does — the
      // turn-end handlers in `ChatRuntimeProvider` clear it again if the turn
      // is cancelled instead. Only on success: a failed decide leaves the call
      // parked, and dropping the prompt would strand the thread until the
      // gate's TTL with nothing left on screen to retry from.
      if (threadId) dispatch(clearPendingApprovalForThread({ threadId }));
    },
    [dispatch, threadId]
  );

  return useMemo(
    () => ({
      messages: runtimeMessages,
      isRunning,
      isLoading,
      extras,
      // Already `ThreadMessageLike`; the runtime's converter is the identity.
      convertMessage: (m: (typeof runtimeMessages)[number]) => m,
      onNew,
      onCancel,
      onRespondToToolApproval,
    }),
    [runtimeMessages, isRunning, isLoading, extras, onNew, onCancel, onRespondToToolApproval]
  );
}
