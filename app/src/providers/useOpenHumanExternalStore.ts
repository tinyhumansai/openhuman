import type {
  AddToolResultOptions,
  AppendMessage,
  ThreadMessage as AuiThreadMessage,
  DictationAdapter,
  RespondToToolApprovalOptions,
  ThreadSuggestion,
} from '@assistant-ui/react';
import { useCallback, useEffect, useMemo, useState } from 'react';

import { useOpenHumanQueueAdapter } from '../features/conversations/aui/queueAdapter';
import { mapDisplayItems } from '../features/conversations/derived/mapDisplayItems';
import { useT } from '../lib/i18n/I18nContext';
import { type ApprovalDecision, decideApproval } from '../services/api/approvalApi';
import { threadApi } from '../services/api/threadApi';
import { editMessage, regenerateMessage } from '../services/chatService';
import {
  clearPendingApprovalForThread,
  type InferenceStatus,
  isActiveTimelineStatus,
  type ToolTimelineEntry,
} from '../store/chatRuntimeSlice';
import { toThreadSuggestions } from '../store/followupSuggestionsSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import {
  FEEDBACK_ROW_IDS_METADATA_KEY,
  persistMessageFeedback,
  truncateMessagesFrom,
} from '../store/threadSlice';
import type { DerivedDisplayItem } from '../types/derivedTranscript';
import type { ThreadMessage } from '../types/thread';
import { buildRuntimeMessages, STREAMING_TAIL_ID } from './assistantUiMessages';
import { getChatSurface } from './chatSurfaceHandlers';
import { openHumanSpeechAdapter } from './speechAdapter';

const EMPTY_MESSAGES: ThreadMessage[] = [];
const EMPTY_SUGGESTIONS: readonly ThreadSuggestion[] = [];
const EMPTY_TIMELINE: never[] = [];
const EMPTY_TRANSCRIPT: never[] = [];
const EMPTY_TURN_MAP = {};
const EMPTY_SETTLED = {};
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
 * Keep the previous array for every turn whose re-projection is unchanged.
 *
 * The projection refetches whenever the thread's last message or lifecycle
 * moves — several times per turn — and each fetch minted fresh arrays for
 * EVERY turn. The settled-message conversion cache is keyed on those array
 * identities, so each refetch re-converted the whole thread and handed
 * assistant-ui new part objects for turns nothing had happened to.
 */
function reuseUnchangedTurns<T>(
  previous: Record<string, T[]>,
  next: Record<string, T[]>
): Record<string, T[]> {
  const nextKeys = Object.keys(next);
  let allReused = nextKeys.length === Object.keys(previous).length;
  const merged: Record<string, T[]> = {};
  for (const key of nextKeys) {
    const before = previous[key];
    if (before !== undefined && JSON.stringify(before) === JSON.stringify(next[key])) {
      merged[key] = before;
    } else {
      merged[key] = next[key];
      allReused = false;
    }
  }
  return allReused ? previous : merged;
}

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
      setProjection(previous => {
        if (previous.threadId !== threadId) {
          return { threadId, timelines: mapped.timelines, transcripts: mapped.transcripts };
        }
        const timelines = reuseUnchangedTurns(previous.timelines, mapped.timelines);
        const transcripts = reuseUnchangedTurns(previous.transcripts, mapped.transcripts);
        if (timelines === previous.timelines && transcripts === previous.transcripts) {
          return previous;
        }
        return { threadId, timelines, transcripts };
      });
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
 * The starter prompts offered on an empty thread, in display order.
 *
 * Each key's translation is used as BOTH the chip's text and the message the
 * chip sends: `ThreadSuggestion.title` falls back to `prompt` upstream
 * (`@assistant-ui/core`'s suggestions client, `title: s.title ?? s.prompt`), so
 * one string per suggestion keeps the chip and the sent turn from ever drifting
 * apart — and keeps this at six translated strings rather than twelve.
 */
const WELCOME_SUGGESTION_KEYS = [
  'chat.welcomeSuggestion.calendarToday',
  'chat.welcomeSuggestion.unreadEmail',
  'chat.welcomeSuggestion.draftReply',
  'chat.welcomeSuggestion.followUps',
  'chat.welcomeSuggestion.connectIntegration',
  'chat.welcomeSuggestion.dailySummaryFlow',
] as const;

/**
 * Starter chips for an empty thread — and nothing once the thread has content.
 *
 * **The emptiness gate is the whole point of this hook, not an optimisation.**
 * The welcome chips and the follow-up chips read the *same* field
 * (`s.thread.suggestions`) and their render gates are complementary rather than
 * overlapping, so between them they partition every state:
 *
 * - welcome  — `isNewChatView && composer.isEmpty`  (`thread.tsx`)
 * - follow-up — `!isEmpty && !isRunning && length > 0` (`follow-up-suggestions.tsx`)
 *
 * They therefore never render at the same time, which is exactly what makes an
 * ungated list dangerous: a constant set shows correctly on the empty thread and
 * then *reappears as follow-up chips under every settled turn, forever*. Static
 * starter prompts hanging under turn 30 are worse than no chips at all.
 *
 * Gating here — at the only inlet — keeps the follow-up surface for what only
 * `useFollowupSuggestions` produces: the core's per-turn `chat_suggestions`
 * (openhuman#6465 records this constraint). Do not lift the gate to the
 * renderer: it cannot tell the two surfaces apart, because they read one field.
 *
 * `messageCount` is the *runtime's* message count (settled turns plus any live
 * tail), which is precisely what `isNewChatView` tests upstream — not the
 * Redux row count, which excludes the in-flight turn and would leave the chips
 * up for the first streaming answer.
 */
function useWelcomeSuggestions(
  messageCount: number,
  enabled: boolean
): readonly ThreadSuggestion[] {
  const { t } = useT();
  return useMemo(
    () =>
      enabled && messageCount === 0
        ? WELCOME_SUGGESTION_KEYS.map(key => ({ prompt: t(key) }))
        : EMPTY_SUGGESTIONS,
    [enabled, messageCount, t]
  );
}

/**
 * The core's follow-up chips for the thread's latest turn, and nothing else.
 *
 * The complement of `useWelcomeSuggestions`: empty on an empty thread (the
 * welcome chips own that state), empty while a turn runs, and empty unless
 * the transcript ends on an assistant reply, because the chips follow that
 * reply. The set comes from `chat_suggestions` via `followupSuggestionsSlice`,
 * which also drops it the moment the next turn starts.
 */
function useFollowupSuggestions(
  threadId: string | null,
  messageCount: number,
  lastRole: string | undefined,
  isRunning: boolean
): readonly ThreadSuggestion[] {
  const stored = useAppSelector(state =>
    threadId ? (state.followupSuggestions?.byThread[threadId] ?? null) : null
  );
  return useMemo(() => {
    if (!stored || messageCount === 0 || isRunning || lastRole !== 'assistant') {
      return EMPTY_SUGGESTIONS;
    }
    return toThreadSuggestions(stored.suggestions);
  }, [stored, messageCount, lastRole, isRunning]);
}

/**
 * The excerpt the user quoted, as a markdown blockquote, or `''`.
 *
 * The composer carries a quote as STRUCTURE — `metadata.custom.quote`, a
 * `{ text, messageId }` set by `SelectionToolbarPrimitive.Quote` — but
 * `surface.send` takes a string, so it has to be rendered into the message or
 * it never reaches the model. Dropping it would leave the quote chip in the
 * composer as decoration: the user would watch themselves quote a paragraph
 * and the agent would answer as though they had not.
 *
 * A blockquote is the representation to pick: it is what the excerpt already
 * is, every model reads it as quoted material, and it survives in the
 * persisted message so the turn still makes sense on reload.
 *
 * `messageId` is deliberately dropped. Nothing downstream can resolve it — the
 * core stores no reference between messages — and a raw id in the prompt is
 * noise to the model.
 */
function appendMessageQuote(message: AppendMessage): string {
  const quote = (message.metadata as { custom?: { quote?: { text?: unknown } } } | undefined)
    ?.custom?.quote;
  const text = typeof quote?.text === 'string' ? quote.text.trim() : '';
  if (text.length === 0) return '';
  return `${text
    .split('\n')
    .map(line => `> ${line}`)
    .join('\n')}\n\n`;
}

/**
 * Build the `ExternalStoreAdapter` that backs `useExternalStoreRuntime`.
 *
 * Settled messages and live deltas remain in their existing UI stores, while
 * reasoning/tool/sub-agent history comes directly from the core transcript
 * projection. Redux is not a second transcript database.
 */
export function useOpenHumanExternalStore(
  threadId: string | null,
  {
    welcomeSuggestions = true,
    dictationAdapter,
  }: {
    /**
     * Offer the home chat's starter prompts on an empty thread. Off for a
     * surface whose agent is not the general assistant (the workflow copilot):
     * a click SENDS the prompt, and those prompts are not builder requests.
     */
    welcomeSuggestions?: boolean;
    /** Configured core STT with a capture lifecycle owned by this runtime. */
    dictationAdapter?: DictationAdapter;
  } = {}
) {
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
  const liveRequestId = useAppSelector(state =>
    threadId ? state.chatRuntime.liveRequestIdByThread?.[threadId] : undefined
  );
  const liveTimelineRequestId = useAppSelector(state =>
    threadId ? state.chatRuntime.toolTimelineRequestByThread?.[threadId] : undefined
  );
  const settledTurns = useAppSelector(state =>
    threadId ? (state.chatRuntime.settledTurnsByThread?.[threadId] ?? EMPTY_SETTLED) : EMPTY_SETTLED
  );
  const settledRevision = `${messages.at(-1)?.id ?? ''}:${messages.at(-1)?.content?.length ?? 0}:${lifecycle ?? ''}`;
  const tailRequestId = liveRequestId ?? streaming?.requestId;
  const coreTranscript = useCoreTranscriptProjection(threadId, settledRevision, tailRequestId);

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
        liveTimelineRequestId,
        pendingApproval,
        turnTimelines: coreTranscript.timelines,
        turnTranscripts: coreTranscript.transcripts,
        settledTurns,
        liveRequestId: tailRequestId,
      }),
    [
      messages,
      streaming,
      isRunning,
      liveTimeline,
      liveTranscript,
      liveTimelineRequestId,
      pendingApproval,
      coreTranscript,
      settledTurns,
      tailRequestId,
    ]
  );

  // The two gates are disjoint (welcome needs an empty thread, follow-ups a
  // settled reply), so at most one of these is ever non-empty.
  const welcomeChips = useWelcomeSuggestions(runtimeMessages.length, welcomeSuggestions);
  const followupChips = useFollowupSuggestions(
    threadId,
    runtimeMessages.length,
    runtimeMessages.at(-1)?.role,
    isRunning
  );
  const suggestions = welcomeChips.length > 0 ? welcomeChips : followupChips;

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
      await surface.send(`${appendMessageQuote(message)}${text}`);
    },
    [threadId]
  );

  /**
   * Thumbs on a settled assistant reply.
   *
   * Supplying this key is what turns the capability on at all — the runtime
   * computes `capabilities.feedback` as `!!adapters?.feedback` and renders
   * nothing without it.
   *
   * `submit` returns `void` by contract: there is no promise for the runtime to
   * await and no error channel back to it. A failed persist therefore cannot
   * surface through the adapter, so the thunk owns the failure, and the
   * optimistic value is only committed by its `fulfilled` reducer — a rejected
   * write leaves the thumb unpressed rather than showing a rating that was never
   * stored.
   */
  const feedbackAdapter = useMemo(
    () => ({
      submit: ({ message, type }: { message: AuiThreadMessage; type: 'positive' | 'negative' }) => {
        // The live tail is not a persisted row; there is nothing to attach a
        // rating to until the turn settles.
        if (!threadId || message.id === STREAMING_TAIL_ID) return;
        const custom = message.metadata?.custom as
          | { extraMetadata?: Record<string, unknown> }
          | undefined;
        const rowIds = custom?.extraMetadata?.[FEEDBACK_ROW_IDS_METADATA_KEY];
        void dispatch(
          persistMessageFeedback({
            threadId,
            // `toThreadMessageLike` carries our own row id through unchanged, and
            // for a merged run that is the LAST row's (see `mergeAssistantRun`).
            messageId: message.id,
            feedback: type,
            rowIds: Array.isArray(rowIds) ? (rowIds as string[]) : undefined,
          })
        );
      },
    }),
    [dispatch, threadId]
  );

  const onCancel = useCallback(async () => {
    await getChatSurface(threadId)?.cancel?.();
  }, [threadId]);

  // The core's run queue, as assistant-ui's message queue. Supplying it makes
  // the runtime send through `queue.enqueue` / `queue.steer` instead of
  // `onNew`; both forward to `onNew`, so the surface still picks the
  // `queue_mode` (see `features/conversations/aui/queueAdapter.ts`).
  const queue = useOpenHumanQueueAdapter(threadId, onNew);

  /**
   * Rewrite a settled message and resend it, via the `threads.edit_message`
   * RPC (wire-contract.md; core workstream C4). `message.sourceId` is
   * assistant-ui's own field for "the id of the message that was edited" —
   * present because `EditComposer`/the vendored `EditMessage` element calls
   * `useAui().thread.append` with the original message's id as `sourceId`.
   *
   * Supplying this key at all is what turns `capabilities.edit` on
   * (`ExternalStoreThreadRuntimeCore` computes it as `!!this._store.onEdit`),
   * which un-gates `UserActionBar`'s Edit button and `EditComposer` in
   * `thread.tsx` (`useAuiEditCapabilities`).
   */
  const onEdit = useCallback(
    async (message: AppendMessage) => {
      if (!threadId) {
        throw new Error('No thread selected for edit');
      }
      const messageId = message.sourceId;
      if (!messageId) {
        throw new Error('Edit is missing the source message id');
      }
      const text = `${appendMessageQuote(message)}${appendMessageText(message)}`;
      // Truncate the local cache FIRST: the edit RPC returns no message list,
      // and the socket events that follow (`inference_start` … `chat_done`)
      // only carry the new turn, so a reader would still see the discarded
      // replies until the next full refetch if this waited on the RPC.
      dispatch(truncateMessagesFrom({ threadId, messageId, inclusive: true }));
      await editMessage({ threadId, messageId, content: text });
    },
    [dispatch, threadId]
  );

  /**
   * Re-run the turn after `parentId` (the assistant message being reloaded,
   * or the message immediately before the point to regenerate from), via the
   * `threads.regenerate` RPC. Same capability-gating rule as `onEdit`:
   * supplying `onReload` is what turns `capabilities.reload` on, which
   * un-gates the Reload button in `AssistantActionBar` (`useAuiReloadCapability`).
   */
  const onReload = useCallback(
    async (parentId: string | null) => {
      if (!threadId) {
        throw new Error('No thread selected for reload');
      }
      if (parentId) {
        dispatch(truncateMessagesFrom({ threadId, messageId: parentId, inclusive: false }));
      }
      await regenerateMessage({ threadId, messageId: parentId ?? undefined });
    },
    [dispatch, threadId]
  );

  /**
   * Required alongside `onEdit`/`onReload` to un-gate `BranchPicker`
   * (`capabilities.switchToBranch` is `!!this._store.setMessages`). A no-op:
   * there is no per-branch message model on the core yet — `onEdit` and
   * `onReload` both truncate the thread's single lineage rather than forking
   * one, so the runtime never has an alternate branch to hand back here.
   */
  const setMessages = useCallback(() => {}, []);

  /**
   * Drop a message from the local cache only — there is no backend RPC to
   * delete a persisted turn.
   *
   * Backs the vendored `StoppedRun` element's Discard action
   * (`components/assistant-ui/thread.tsx`): the partial reply a stopped turn
   * persists (`extraMetadata.stopped`, `Conversations.tsx`) is real content
   * server-side, so this hides it from THIS client rather than erasing it —
   * the same "never erases, only trims what the client reads" posture the
   * transcript takes on compaction.
   *
   * Supplying `onDelete` at all is what the runtime checks FIRST
   * (`ExternalStoreThreadRuntimeCore.deleteMessage`), ahead of the
   * `setMessages`-based fallback that already made `capabilities.delete`
   * true. That fallback filters its own internal repository and hands the
   * result to `setMessages`, which above is a no-op — so without this, a
   * `message.delete()` call would flash the message away and then restore it
   * on the next render, since `messages` here is still bound to the
   * unmodified Redux array. Reusing `truncateMessagesFrom` (the same local
   * cache trim `onEdit`/`onReload` use) is what actually removes it.
   */
  const onDelete = useCallback(
    (messageId: string) => {
      if (!threadId) return;
      dispatch(truncateMessagesFrom({ threadId, messageId, inclusive: true }));
    },
    [dispatch, threadId]
  );

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

  /**
   * Answer a structured human-input request the run is parked on
   * (`ask_user_clarification`, and any WS-D sub-agent clarification that
   * reuses `ElicitationAdapter`).
   *
   * Every OpenHuman tool is a `type: 'backend'` toolkit entry (`aui/
   * toolkit.tsx`) — the core executes it, never the browser — so there is no
   * "resolve this call with a client-computed result" RPC for
   * `onAddToolResult` to call. What unblocks the parked call is the SAME
   * mechanism `ChatToolParts.tsx`'s `SubagentCall.onAnswer` already uses for
   * the sub-agent case: an ordinary next turn through the registered chat
   * surface, which the core's orchestrator treats as the clarification
   * reply. Supplying this key is what turns `onAddToolResult` into a real
   * capability rather than a throw the moment `ElicitationAdapter`'s Send
   * button is wired to it.
   */
  const onAddToolResult = useCallback(
    async ({ result }: AddToolResultOptions) => {
      const surface = getChatSurface(threadId);
      if (!surface) return;
      const text = typeof result === 'string' ? result : JSON.stringify(result);
      if (text.trim().length === 0) return;
      await surface.send(text);
    },
    [threadId]
  );

  /** Same rationale as `onAddToolResult` above, for a resumed (paused) call. */
  const onResumeToolCall = useCallback(
    async ({ payload }: { toolCallId: string; payload: unknown }) => {
      const surface = getChatSurface(threadId);
      if (!surface) return;
      const text = typeof payload === 'string' ? payload : JSON.stringify(payload);
      if (text.trim().length === 0) return;
      await surface.send(text);
    },
    [threadId]
  );

  // Inline dictation uses MediaRecorder and the configured core STT provider.
  // The runtime appends its final transcript to the editable draft.
  return useMemo(
    () => ({
      messages: runtimeMessages,
      isRunning,
      isLoading,
      extras,
      // Welcome chips on an empty thread, the core's follow-ups after a settled
      // reply, otherwise empty — see `useWelcomeSuggestions`.
      suggestions,
      // Already `ThreadMessageLike`; the runtime's converter is the identity.
      convertMessage: (m: (typeof runtimeMessages)[number]) => m,
      onNew,
      onCancel,
      queue,
      onEdit,
      onReload,
      setMessages,
      onDelete,
      onRespondToToolApproval,
      onAddToolResult,
      onResumeToolCall,
      // Read-aloud for a single message. Supplying this is what makes
      // `capabilities.speech` true and the Speak / StopSpeaking controls
      // usable — and it must ship WITH the buttons, never before or after
      // them: `actionBarSpeakDisabled` does not consult the capability (it
      // checks only role and running status), so a Speak button rendered
      // without an adapter is enabled, clickable, and throws "Runtime does not
      // support speech." That is the #5897 defect shape, and Reload already
      // sits in the same trap today.
      adapters: {
        feedback: feedbackAdapter,
        speech: openHumanSpeechAdapter,
        dictation: dictationAdapter,
      },
    }),
    [
      runtimeMessages,
      isRunning,
      isLoading,
      extras,
      suggestions,
      feedbackAdapter,
      dictationAdapter,
      onNew,
      onCancel,
      queue,
      onEdit,
      onReload,
      setMessages,
      onDelete,
      onRespondToToolApproval,
      onAddToolResult,
      onResumeToolCall,
    ]
  );
}
