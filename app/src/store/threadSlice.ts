import { createAsyncThunk, createSlice, type PayloadAction } from '@reduxjs/toolkit';

import { extractWorkflowProposalFromMessages } from '../lib/workflows/workflowProposal';
import { threadApi } from '../services/api/threadApi';
import type { ChatChannelEventEvent, ContentPart } from '../services/chatWire';
import { isThreadNotFoundCoreRpcError } from '../services/coreRpcClient';
import type { Thread, ThreadMessage } from '../types/thread';
import { IS_DEV } from '../utils/config';
import { jsonValuesEqual } from '../utils/jsonValuesEqual';
import { setWorkflowProposalForThread } from './chatRuntimeSlice';
import { resetUserScopedState } from './resetActions';

export const THREAD_NOT_FOUND_MESSAGE = 'This thread is no longer available.';

interface ThreadState {
  threads: Thread[];
  selectedThreadId: string | null;
  /** Incremented by every explicit thread selection to invalidate stale async continuations. */
  selectionIntentVersion: number;
  /**
   * Set of threads that currently have an in-flight inference turn, keyed by
   * thread id. Replaces the legacy single `activeThreadId` so that turns on
   * different threads can run concurrently — each thread tracks its own
   * in-flight state and the composer is gated per-thread (see
   * `isComposerInteractionBlocked`) rather than globally. Stored as a plain
   * object (not a `Set`) to stay redux-persist serializable.
   */
  activeThreadIds: Record<string, true>;
  /**
   * @deprecated — welcome-agent replaced by Joyride walkthrough. Always null
   * for new users; retained for redux-persist deserialization compatibility.
   */
  welcomeThreadId: string | null;
  messagesByThreadId: Record<string, ThreadMessage[]>;
  /** Live provider receipts/reactions keyed by exact persisted thread/message IDs. */
  channelActivityByThreadId: Record<string, Record<string, ChannelActivity>>;
  messages: ThreadMessage[];
  isLoadingThreads: boolean;
  isLoadingMessages: boolean;
  messagesError: string | null;
  /**
   * Why the last `createNewThread` failed, or `null` when the last attempt
   * succeeded (or none has run). Recorded centrally so a failed create is never
   * invisible: before #5156 nothing in the store observed
   * `createNewThread.rejected`, so every call site had to remember its own
   * `.catch` — one that forgot produced `UnhandledRejection: Core RPC
   * openhuman.threads_create_new timed out after 30000ms` (Sentry
   * TAURI-REACT-10) and one that caught-and-ignored left the user staring at a
   * dead "New chat" button. The chat surface renders this.
   */
  createThreadError: string | null;
  /**
   * `requestId` of the most recently *started* `createNewThread`, so a
   * completion from a superseded attempt can be ignored.
   *
   * Two creates can overlap — the user hits New Chat again while the first RPC
   * is still hung on its 30 s budget. Without this, the newer request could
   * succeed and clear the banner, and then the older one would time out and set
   * `createThreadError` again, showing "Couldn't create a new thread" on a chat
   * that was in fact created. Only the latest attempt is allowed to write this
   * slice's create-error state.
   */
  createThreadRequestId: string | null;
}

interface ChannelActivity {
  reactions: Array<{ actorId: string; value: string; customEmojiId?: string }>;
  readReceipts: Array<{ actorId: string; timestamp?: number }>;
}

const initialState: ThreadState = {
  threads: [],
  selectedThreadId: null,
  selectionIntentVersion: 0,
  activeThreadIds: {},
  welcomeThreadId: null,
  messagesByThreadId: {},
  channelActivityByThreadId: {},
  messages: [],
  isLoadingThreads: false,
  isLoadingMessages: false,
  messagesError: null,
  createThreadError: null,
  createThreadRequestId: null,
};

/**
 * Append a persisted message to the thread cache, or replace the entry that
 * already carries its id.
 *
 * Ids are no longer guaranteed fresh on append: a core-initiated turn's reply
 * is persisted by the core under `agent:<run_id>` and then persisted again by
 * our `chat_done` handler under the same id (the store collapses the two —
 * #5933). If a `loadThreadMessages` fetch lands between those two writes, the
 * cache already holds that id when the append resolves, and a second entry
 * would hand React and assistant-ui a duplicate key (assistant-ui throws on
 * one). `replaceExisting` keeps its narrower contract for callers that only
 * ever update a message already in the cache (reactions).
 */
function appendMessageToCache(
  state: ThreadState,
  threadId: string,
  message: ThreadMessage,
  replaceExisting = false
) {
  const upsert = (list: ThreadMessage[]) => {
    const present = list.some(e => e.id === message.id);
    if (present && list.some(e => e.id === message.id && jsonValuesEqual(e, message))) return list;
    if (replaceExisting || present) {
      return list.map(e => (e.id === message.id ? message : e));
    }
    return [...list, message];
  };
  state.messagesByThreadId[threadId] = upsert(state.messagesByThreadId[threadId] ?? []);
  if (threadId === state.selectedThreadId) {
    state.messages = upsert(state.messages);
  }
}

// ── Async thunks (thin RPC wrappers) ──────────────────────────────

export const loadThreads = createAsyncThunk<
  Awaited<ReturnType<typeof threadApi.getThreads>> & { selectionIntentVersionAtRequest?: number },
  void
>('thread/loadThreads', async (_, { getState, rejectWithValue }) => {
  const selectionIntentVersionAtRequest =
    (getState() as { thread?: Pick<ThreadState, 'selectionIntentVersion'> }).thread
      ?.selectionIntentVersion ?? 0;
  try {
    return { ...(await threadApi.getThreads()), selectionIntentVersionAtRequest };
  } catch (error) {
    return rejectWithValue(error instanceof Error ? error.message : 'Failed to load threads');
  }
});

/**
 * Normalise whatever `dispatch(createNewThread()).unwrap()` throws into a
 * displayable string.
 *
 * `createAsyncThunk` throws the `rejectWithValue` payload (a bare string) or, if
 * the thunk threw, Redux's `SerializedError` — a plain object, not an `Error`.
 * Neither carries a stack, which is why the original report surfaced as
 * "Non-Error promise rejection captured with value: …" (#5156). Mirrors
 * `formatThreadLoadError` in `features/conversations/Conversations.tsx`.
 */
export function formatThreadCreateError(error: unknown): string {
  if (typeof error === 'string' && error.trim().length > 0) return error;
  // `new Error()` carries an empty `message`. Returning it would store a falsy
  // `createThreadError`, which `deriveChatErrorBanner` treats as "no failure" —
  // the user would get a dead New Chat button and no banner, the exact outcome
  // #5156 exists to prevent. Fall through to the generic fallback instead.
  if (error instanceof Error && error.message.trim().length > 0) return error.message;
  if (error && typeof error === 'object' && 'message' in error) {
    const message = (error as { message?: unknown }).message;
    if (typeof message === 'string' && message.trim().length > 0) return message;
  }
  return 'Failed to create thread';
}

export const createNewThread = createAsyncThunk(
  'thread/createNewThread',
  async (labels: string[] | undefined, { dispatch, rejectWithValue }) => {
    try {
      const thread = await threadApi.createNewThread(labels);
      await dispatch(loadThreads()).unwrap();
      return thread;
    } catch (error) {
      // Keep the payload a plain string (redux-serializable) but normalise every
      // throw shape, including the `SerializedError` a nested `.unwrap()` raises
      // when `loadThreads` is what actually failed (#5156).
      return rejectWithValue(formatThreadCreateError(error));
    }
  }
);

export const deleteThread = createAsyncThunk(
  'thread/deleteThread',
  async (threadId: string, { dispatch, getState, rejectWithValue }) => {
    try {
      await threadApi.deleteThread(threadId);
      const state = getState() as { thread: ThreadState };
      if (state.thread.selectedThreadId === threadId) {
        const remaining = state.thread.threads.filter(t => t.id !== threadId);
        if (remaining.length > 0) {
          dispatch(setSelectedThread(remaining[0].id));
        } else {
          dispatch(clearSelectedThread());
        }
      }
      await dispatch(loadThreads()).unwrap();
      return { threadId };
    } catch (error) {
      return rejectWithValue(error instanceof Error ? error.message : 'Failed to delete thread');
    }
  }
);

export const loadThreadMessages = createAsyncThunk(
  'thread/loadThreadMessages',
  async (threadId: string, { dispatch, rejectWithValue }) => {
    try {
      const response = await threadApi.getThreadMessages(threadId);
      // Durable workflow-proposal rehydration: the Rust core persists a
      // proposal produced by an async workflow_builder run as a thread
      // message (extraMetadata.scope === 'workflow_proposal'). Restoring it
      // here means the proposal card survives reloads, reconnects, and
      // dropped socket events — the live `tool_result` events remain the
      // fast path and simply overwrite this with the same payload.
      const persistedProposal = extractWorkflowProposalFromMessages(response.messages);
      if (persistedProposal) {
        dispatch(setWorkflowProposalForThread({ threadId, proposal: persistedProposal }));
      }
      return { threadId, messages: response.messages };
    } catch (error) {
      // A cached/seeded thread id (e.g. the Flows copilot's persisted
      // `workflowCopilotThreads.ts` mapping) can point at a thread that was
      // since deleted/purged. Evict it the same way the write thunks below
      // do, so callers see a clean rejection instead of permanently retrying
      // a dead thread — `useWorkflowBuilderChat`'s rehydrate effect uses this
      // to null out its `threadId` and let the next send start a fresh one.
      if (isThreadNotFoundCoreRpcError(error, threadId)) {
        await evictStaleThread(threadId, dispatch);
        return rejectWithValue(THREAD_NOT_FOUND_MESSAGE);
      }
      return rejectWithValue(error instanceof Error ? error.message : 'Failed to load messages');
    }
  }
);

/**
 * Shared stale-thread handler used by write thunks.
 *
 * When `isThreadNotFoundCoreRpcError` is true the thunk should:
 *   1. Evict the stale thread from Redux state immediately.
 *   2. Reload the thread list so the sidebar reflects backend state.
 *   3. Reject with `THREAD_NOT_FOUND_MESSAGE` so callers can branch on it
 *      without inspecting error message strings.
 *
 * The `loadThreads` failure is swallowed — a network hiccup on the refresh
 * should not surface an additional error on top of the stale-thread UX.
 */
/**
 * Side-effect half of stale-thread cleanup: evict the thread from Redux state
 * and reload the thread list. Separated from the `rejectWithValue` call so the
 * thunk return type is inferred purely from `rejectWithValue(THREAD_NOT_FOUND_MESSAGE)`.
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function evictStaleThread(threadId: string, dispatch: (action: any) => any): Promise<void> {
  dispatch(clearStaleThread(threadId));
  try {
    await dispatch(loadThreads()).unwrap();
  } catch (refreshError) {
    if (IS_DEV) {
      console.debug('[threadSlice] stale-thread list refresh failed', {
        threadId,
        error: refreshError,
      });
    }
  }
}

export const addMessageLocal = createAsyncThunk(
  'thread/addMessageLocal',
  async (
    payload: { threadId: string; message: ThreadMessage; attachments?: ContentPart[] },
    { dispatch, rejectWithValue }
  ) => {
    try {
      const persisted = await threadApi.appendMessage(
        payload.threadId,
        payload.message,
        payload.attachments
      );
      if (payload.message.sender === 'user' && payload.message.content.trim()) {
        void dispatch(generateThreadTitleIfNeeded({ threadId: payload.threadId }))
          .unwrap()
          .catch(error => {
            if (IS_DEV) {
              console.debug('[threadSlice] addMessageLocal title refresh failed', {
                threadId: payload.threadId,
                error,
              });
            }
          });
      }
      return { threadId: payload.threadId, message: persisted };
    } catch (error) {
      if (isThreadNotFoundCoreRpcError(error, payload.threadId)) {
        await evictStaleThread(payload.threadId, dispatch);
        return rejectWithValue(THREAD_NOT_FOUND_MESSAGE);
      }
      return rejectWithValue(error instanceof Error ? error.message : 'Failed to save message');
    }
  }
);

export const addInferenceResponse = createAsyncThunk(
  'thread/addInferenceResponse',
  async (
    payload: {
      content: string;
      threadId?: string;
      messageId?: string;
      type?: string;
      extraMetadata?: Record<string, unknown>;
    },
    { dispatch, getState, rejectWithValue }
  ) => {
    const state = getState() as { thread: ThreadState };
    // All real callers pass an explicit `threadId` (the event's `thread_id`).
    // The fallback to the selected thread is a last-resort for legacy callers
    // that omit it; under parallel inference there is no single "active" thread
    // to fall back to, so we cannot use `activeThreadIds` here.
    const targetThreadId = payload.threadId ?? state.thread.selectedThreadId;
    if (!targetThreadId) return rejectWithValue('No target thread');

    const message: ThreadMessage = {
      id:
        payload.messageId ??
        `msg_${globalThis.crypto?.randomUUID ? globalThis.crypto.randomUUID() : `${Date.now()}-${Math.random().toString(36).slice(2)}`}`,
      content: payload.content,
      type: payload.type ?? 'text',
      extraMetadata: payload.extraMetadata ?? {},
      sender: 'agent',
      createdAt: new Date().toISOString(),
    };

    try {
      const persisted = await threadApi.appendMessage(targetThreadId, message);
      return { threadId: targetThreadId, message: persisted };
    } catch (error) {
      if (isThreadNotFoundCoreRpcError(error, targetThreadId)) {
        await evictStaleThread(targetThreadId, dispatch);
        return rejectWithValue(THREAD_NOT_FOUND_MESSAGE);
      }
      return rejectWithValue(error instanceof Error ? error.message : 'Failed to save response');
    }
  }
);

export const generateThreadTitleIfNeeded = createAsyncThunk(
  'thread/generateThreadTitleIfNeeded',
  async (
    payload: { threadId: string; assistantMessage?: string },
    { dispatch, rejectWithValue }
  ) => {
    let thread: Thread;
    try {
      thread = await threadApi.generateTitleIfNeeded(payload.threadId, payload.assistantMessage);
    } catch (error) {
      if (isThreadNotFoundCoreRpcError(error, payload.threadId)) {
        await evictStaleThread(payload.threadId, dispatch);
        return rejectWithValue(THREAD_NOT_FOUND_MESSAGE);
      }
      return rejectWithValue(
        error instanceof Error ? error.message : 'Failed to generate thread title'
      );
    }

    try {
      await dispatch(loadThreads()).unwrap();
    } catch (error) {
      if (IS_DEV) {
        console.debug('[threadSlice] generateThreadTitleIfNeeded refresh failed', {
          threadId: payload.threadId,
          error,
        });
      }
    }

    return thread;
  }
);

/** The rating a user gave one assistant reply. */
export type MessageFeedback = 'positive' | 'negative';

/** `extraMetadata` key holding the rating. */
export const FEEDBACK_METADATA_KEY = 'feedback';

/**
 * `extraMetadata` key holding every persisted row id that made up the assistant
 * run the user actually rated.
 *
 * One visible assistant message is often several persisted rows (see
 * `mergeAssistantRun`), and the adapter only ever hands us the last row's id.
 * Recording the whole set means the rating stays attributable to what was on
 * screen rather than to the fragment that happened to be last.
 */
export const FEEDBACK_ROW_IDS_METADATA_KEY = 'feedbackRowIds';

/**
 * `extraMetadata` key holding the turn's latency snapshot
 * (`ChatDoneEvent.timing` — wire-contract.md), stamped by
 * `ChatRuntimeProvider`'s `chatDoneExtraMetadata` and read back by
 * `assistantUiMessages.ts` to build `ThreadMessageLike.metadata.timing` for
 * the vendored `MessageTiming` element.
 */
export const TIMING_METADATA_KEY = 'timing';

/**
 * `extraMetadata` key holding a `chat_error`'s `error_type` (and, for
 * `"guardrail"`, its `GuardrailPayload`) — wire-contract.md. Stamped by
 * `ChatRuntimeProvider`'s `onError` handler on the assistant message it
 * appends for the failed turn; read back by `assistantUiMessages.ts` and
 * `ChatErrorNotice` (`features/conversations/aui/`) to render the vendored
 * `GuardrailNotice` element in place of the plain error text.
 */
export const CHAT_ERROR_METADATA_KEY = 'chatError';

/**
 * Persist a thumbs rating on one assistant message: read the row from Redux,
 * patch its `extraMetadata`, and write back the persisted row.
 *
 * Pressing the same rating again clears it, so a mis-click is recoverable: the
 * assistant-ui action bar has no third "unrated" control to offer.
 */
export const persistMessageFeedback = createAsyncThunk(
  'thread/persistMessageFeedback',
  async (
    payload: {
      threadId: string;
      messageId: string;
      feedback: MessageFeedback;
      /** Row ids of the merged run, when the rated message spans several. */
      rowIds?: readonly string[];
    },
    { getState, rejectWithValue }
  ) => {
    const state = getState() as { thread: ThreadState };
    const stored = state.thread.messagesByThreadId[payload.threadId] ?? [];
    const message = stored.find(e => e.id === payload.messageId);
    if (!message) return rejectWithValue('Message not found');

    const currentMetadata = message.extraMetadata ?? {};
    const previous = currentMetadata[FEEDBACK_METADATA_KEY];
    const next = previous === payload.feedback ? undefined : payload.feedback;
    const extraMetadata: Record<string, unknown> = {
      ...currentMetadata,
      [FEEDBACK_METADATA_KEY]: next,
    };
    if (payload.rowIds && payload.rowIds.length > 1) {
      extraMetadata[FEEDBACK_ROW_IDS_METADATA_KEY] = [...payload.rowIds];
    }

    try {
      const persisted = await threadApi.updateMessage(
        payload.threadId,
        payload.messageId,
        extraMetadata
      );
      return { threadId: payload.threadId, message: persisted };
    } catch (error) {
      return rejectWithValue(error instanceof Error ? error.message : 'Failed to save feedback');
    }
  }
);

export const updateThreadTitle = createAsyncThunk(
  'thread/updateThreadTitle',
  async (payload: { threadId: string; title: string }, { rejectWithValue }) => {
    try {
      return await threadApi.updateTitle(payload.threadId, payload.title);
    } catch (error) {
      return rejectWithValue(
        error instanceof Error ? error.message : 'Failed to update thread title'
      );
    }
  }
);

export const updateThreadWorkingDir = createAsyncThunk(
  'thread/updateThreadWorkingDir',
  async (payload: { threadId: string; actionDir: string | null }, { rejectWithValue }) => {
    try {
      return await threadApi.updateWorkingDir(payload.threadId, payload.actionDir ?? '');
    } catch (error) {
      return rejectWithValue(
        error instanceof Error ? error.message : 'Failed to update the working folder'
      );
    }
  }
);

// ── Slice ─────────────────────────────────────────────────────────

const threadSlice = createSlice({
  name: 'thread',
  initialState,
  reducers: {
    applyChannelEvent(state, action: PayloadAction<ChatChannelEventEvent>) {
      const event = action.payload;
      const rowId = event.openhuman_message_id;
      if (!rowId || !event.thread_id) return;
      state.channelActivityByThreadId ??= {};
      const thread = (state.channelActivityByThreadId[event.thread_id] ??= {});
      if (!thread[rowId]) {
        const prior = state.messagesByThreadId[event.thread_id]?.find(
          message => message.id === rowId
        );
        const persisted = prior?.extraMetadata?.channelActivity;
        const saved = persisted && typeof persisted === 'object' ? persisted : undefined;
        const storedItems = (items: unknown): Array<Record<string, unknown>> => {
          if (Array.isArray(items)) {
            return items.filter(item => item && typeof item === 'object');
          }
          if (items && typeof items === 'object') {
            return Object.values(items).filter(item => item && typeof item === 'object');
          }
          return [];
        };
        thread[rowId] = {
          reactions: storedItems((saved as ChannelActivity | undefined)?.reactions).map(item => ({
            actorId: typeof item.actorId === 'string' ? item.actorId : '',
            value: typeof item.value === 'string' ? item.value : '',
            ...(typeof item.customEmojiId === 'string'
              ? { customEmojiId: item.customEmojiId }
              : {}),
          })),
          readReceipts: storedItems((saved as ChannelActivity | undefined)?.readReceipts).map(
            item => ({
              actorId: typeof item.actorId === 'string' ? item.actorId : '',
              ...(typeof item.timestamp === 'number' ? { timestamp: item.timestamp } : {}),
            })
          ),
        };
      }
      const activity = thread[rowId];
      const providerEvent = event.channel_event;
      if (providerEvent.type === 'reaction_added') {
        const reaction = {
          actorId: providerEvent.actor.id,
          value: providerEvent.reaction.value,
          ...(providerEvent.reaction.customEmojiId
            ? { customEmojiId: providerEvent.reaction.customEmojiId }
            : {}),
        };
        if (
          !activity.reactions.some(
            item =>
              item.actorId === reaction.actorId &&
              item.value === reaction.value &&
              item.customEmojiId === reaction.customEmojiId
          )
        ) {
          activity.reactions.push(reaction);
        }
      } else if (providerEvent.type === 'reaction_removed') {
        activity.reactions = activity.reactions.filter(
          item =>
            item.actorId !== providerEvent.actor.id ||
            item.value !== providerEvent.reaction.value ||
            item.customEmojiId !== (providerEvent.reaction.customEmojiId ?? undefined)
        );
      } else if (providerEvent.type === 'read_receipt') {
        const receipt = {
          actorId: providerEvent.actor?.id ?? '',
          ...(providerEvent.timestamp == null ? {} : { timestamp: providerEvent.timestamp }),
        };
        const prior = activity.readReceipts.findIndex(item => item.actorId === receipt.actorId);
        if (prior >= 0) activity.readReceipts[prior] = receipt;
        else activity.readReceipts.push(receipt);
      }
      const updateMessage = (message: ThreadMessage) => {
        if (message.id !== rowId) return;
        message.extraMetadata = {
          ...(message.extraMetadata ?? {}),
          channelActivity: { reactions: activity.reactions, readReceipts: activity.readReceipts },
        };
      };
      state.messagesByThreadId[event.thread_id]?.forEach(updateMessage);
      if (state.selectedThreadId === event.thread_id) state.messages.forEach(updateMessage);
    },
    /**
     * Dismiss the "couldn't start a new chat" surface (#5156) — the user has
     * seen it and moved on (typed into the composer, navigated away).
     */
    clearCreateThreadError: state => {
      state.createThreadError = null;
    },
    setSelectedThread: (state, action: { payload: string }) => {
      state.selectionIntentVersion = (state.selectionIntentVersion ?? 0) + 1;
      state.selectedThreadId = action.payload;
      state.messages = state.messagesByThreadId[action.payload] ?? [];
      state.messagesError = null;
    },
    invalidateThreadSelection: state => {
      state.selectionIntentVersion = (state.selectionIntentVersion ?? 0) + 1;
    },
    clearSelectedThread: state => {
      state.selectionIntentVersion = (state.selectionIntentVersion ?? 0) + 1;
      state.selectedThreadId = null;
      state.messages = [];
      state.messagesError = null;
    },
    /**
     * Back-compat shim retained for existing call sites:
     *  - `setActiveThread(threadId)` marks that thread's inference turn active.
     *  - `setActiveThread(null)` clears ALL in-flight markers (legacy global
     *    clear). Prefer `clearThreadInferenceActive(threadId)` when the
     *    finishing thread is known so unrelated concurrent turns are untouched.
     */
    setActiveThread: (state, action: { payload: string | null }) => {
      if (action.payload === null) {
        state.activeThreadIds = {};
      } else {
        state.activeThreadIds[action.payload] = true;
      }
    },
    /** Mark a single thread as having an in-flight inference turn. */
    markThreadInferenceActive: (state, action: PayloadAction<string>) => {
      state.activeThreadIds[action.payload] = true;
    },
    /** Clear the in-flight marker for a single thread (on done/error/timeout). */
    clearThreadInferenceActive: (state, action: PayloadAction<string>) => {
      delete state.activeThreadIds[action.payload];
    },
    clearStaleThread: (state, action: PayloadAction<string>) => {
      const threadId = action.payload;
      state.threads = state.threads.filter(thread => thread.id !== threadId);
      delete state.messagesByThreadId[threadId];
      if (state.channelActivityByThreadId) delete state.channelActivityByThreadId[threadId];
      if (state.selectedThreadId === threadId) {
        state.selectionIntentVersion = (state.selectionIntentVersion ?? 0) + 1;
        state.selectedThreadId = null;
        state.messages = [];
        state.messagesError = null;
      }
      delete state.activeThreadIds[threadId];
      if (state.welcomeThreadId === threadId) {
        state.welcomeThreadId = null;
      }
    },
    clearAllThreads: state => {
      state.selectionIntentVersion = (state.selectionIntentVersion ?? 0) + 1;
      state.threads = [];
      state.messagesByThreadId = {};
      state.channelActivityByThreadId = {};
      state.selectedThreadId = null;
      state.messages = [];
      state.activeThreadIds = {};
      state.welcomeThreadId = null;
    },
    // Like `clearAllThreads` but keeps `selectedThreadId`. Used on the
    // post-bootstrap identity-observation path (#1168 + #1157): we need to
    // drop pre-auth in-memory thread rows but the persisted last-viewed
    // thread id is still valid for the reloaded user, so preserving it lets
    // the Conversations effect resume that thread instead of falling
    // through to "most recent".
    resetThreadCachesPreservingSelection: state => {
      state.threads = [];
      state.messagesByThreadId = {};
      state.channelActivityByThreadId = {};
      state.messages = [];
      state.activeThreadIds = {};
      state.welcomeThreadId = null;
    },
    // [#1123] True no-op — welcome-agent onboarding replaced by Joyride walkthrough.
    // Kept to avoid breaking existing imports; state.welcomeThreadId is never
    // mutated because the welcome-agent flow no longer runs.
    setWelcomeThreadId: () => {
      // intentional no-op
    },
    /**
     * Drop messages at or after `messageId` from `threadId`'s local cache.
     *
     * Backs assistant-ui's `onEdit`/`onReload` (`useOpenHumanExternalStore`):
     * both RPCs (`threads.edit_message`, `threads.regenerate`) truncate the
     * core's own transcript and re-run from that point, but neither returns a
     * fresh message list — the socket events that follow only carry the NEW
     * turn. Without this, the discarded replies would stay visible in the
     * Redux cache until the next full `loadThreadMessages` refetch.
     *
     * `inclusive` distinguishes the two callers: an edit resends `messageId`
     * itself (drop it too), a reload keeps the parent message and only drops
     * what came after it.
     */
    truncateMessagesFrom: (
      state,
      action: PayloadAction<{ threadId: string; messageId: string; inclusive: boolean }>
    ) => {
      const { threadId, messageId, inclusive } = action.payload;
      const existing = state.messagesByThreadId[threadId];
      if (!existing) return;
      const idx = existing.findIndex(m => m.id === messageId);
      if (idx < 0) return;
      const truncated = existing.slice(0, inclusive ? idx : idx + 1);
      state.messagesByThreadId[threadId] = truncated;
      if (state.selectedThreadId === threadId) {
        state.messages = truncated;
      }
    },
    /**
     * Drop exactly these rows from a thread's cache. Unlike
     * `truncateMessagesFrom`, rows appended after the caller took its
     * snapshot survive — a regenerate that resolves after its new turn has
     * already streamed in must not take the new reply with it.
     */
    removeMessagesById: (
      state,
      action: PayloadAction<{ threadId: string; messageIds: readonly string[] }>
    ) => {
      const { threadId, messageIds } = action.payload;
      const existing = state.messagesByThreadId[threadId];
      if (!existing || messageIds.length === 0) return;
      const doomed = new Set(messageIds);
      const kept = existing.filter(m => !doomed.has(m.id));
      state.messagesByThreadId[threadId] = kept;
      if (state.selectedThreadId === threadId) {
        state.messages = kept;
      }
    },
  },
  extraReducers: builder => {
    builder
      .addCase(loadThreads.pending, state => {
        state.isLoadingThreads = true;
      })
      .addCase(loadThreads.fulfilled, (state, action) => {
        state.isLoadingThreads = false;
        const liveThreadIds = new Set(action.payload.threads.map(thread => thread.id));
        const currentIntentVersion = state.selectionIntentVersion ?? 0;
        const selectionWasSuperseded =
          currentIntentVersion !==
          (action.payload.selectionIntentVersionAtRequest ?? currentIntentVersion);
        if (selectionWasSuperseded && !state.selectedThreadId) {
          return;
        }
        const supersedingThread =
          selectionWasSuperseded &&
          state.selectedThreadId &&
          !liveThreadIds.has(state.selectedThreadId)
            ? state.threads.find(thread => thread.id === state.selectedThreadId)
            : undefined;
        if (supersedingThread) liveThreadIds.add(supersedingThread.id);
        state.threads = supersedingThread
          ? [supersedingThread, ...action.payload.threads]
          : action.payload.threads;
        if (
          !selectionWasSuperseded &&
          state.selectedThreadId &&
          !liveThreadIds.has(state.selectedThreadId)
        ) {
          state.selectedThreadId = null;
          state.messages = [];
          state.messagesError = null;
        }
        for (const activeId of Object.keys(state.activeThreadIds)) {
          if (!liveThreadIds.has(activeId)) {
            delete state.activeThreadIds[activeId];
          }
        }
      })
      .addCase(loadThreads.rejected, state => {
        state.isLoadingThreads = false;
      })
      // A create in flight supersedes the previous attempt's failure, so the
      // banner clears the moment the user retries rather than lingering. The
      // starting request also becomes the only one allowed to write the error
      // state — see `createThreadRequestId`.
      .addCase(createNewThread.pending, (state, action) => {
        state.createThreadRequestId = action.meta.requestId;
        state.createThreadError = null;
      })
      .addCase(createNewThread.fulfilled, (state, action) => {
        if (action.meta.requestId !== state.createThreadRequestId) return;
        state.createThreadError = null;
      })
      .addCase(createNewThread.rejected, (state, action) => {
        // A superseded attempt failing late must not repaint the banner over a
        // newer create that already succeeded.
        if (action.meta.requestId !== state.createThreadRequestId) return;
        state.createThreadError = formatThreadCreateError(action.payload ?? action.error?.message);
      })
      .addCase(loadThreadMessages.pending, state => {
        state.isLoadingMessages = true;
        state.messagesError = null;
      })
      .addCase(loadThreadMessages.fulfilled, (state, action) => {
        state.isLoadingMessages = false;
        const { threadId, messages: fetched } = action.payload;
        const existing = state.messagesByThreadId[threadId] ?? [];
        const existingById = new Map(existing.map(message => [message.id, message]));
        const stableFetched = fetched.map(message => {
          const before = existingById.get(message.id);
          return before && jsonValuesEqual(before, message) ? before : message;
        });
        const fetchedIds = new Set(fetched.map(m => m.id));
        // A message present locally but missing from this fetch already
        // persisted server-side (cache entries only ever land via a
        // `.fulfilled` append) — it just hasn't shown up in this particular
        // GET yet (e.g. a `loadThreadMessages` rehydrate racing a concurrent
        // `addMessageLocal`/`addInferenceResponse` append on the same
        // thread). Merge it back in instead of wholesale-replacing, or the
        // fetch would silently wipe the newer message out of the visible
        // transcript. See `useWorkflowBuilderChat`'s copilot-thread rehydrate
        // effect, which can run concurrently with an auto-sent turn.
        const localOnly = existing.filter(m => !fetchedIds.has(m.id));
        const messages =
          localOnly.length > 0
            ? [...stableFetched, ...localOnly].sort(
                (a, b) => Date.parse(a.createdAt) - Date.parse(b.createdAt)
              )
            : stableFetched;
        const stableMessages =
          messages.length === existing.length &&
          messages.every((message, index) => message === existing[index])
            ? existing
            : messages;
        state.messagesByThreadId[threadId] = stableMessages;
        if (threadId === state.selectedThreadId) {
          state.messages = stableMessages;
        }
      })
      .addCase(loadThreadMessages.rejected, (state, action) => {
        state.isLoadingMessages = false;
        state.messagesError = action.payload as string;
      })
      .addCase(addMessageLocal.fulfilled, (state, action) => {
        appendMessageToCache(state, action.payload.threadId, action.payload.message);
      })
      .addCase(generateThreadTitleIfNeeded.fulfilled, (state, action) => {
        const idx = state.threads.findIndex(thread => thread.id === action.payload.id);
        if (idx >= 0) {
          state.threads[idx] = action.payload;
        } else {
          state.threads = [action.payload, ...state.threads];
        }
      })
      .addCase(addInferenceResponse.fulfilled, (state, action) => {
        appendMessageToCache(state, action.payload.threadId, action.payload.message);
        // Do not clear activeThreadId here: streaming sends many segment append
        // thunks; clearing each time would re-enable the composer mid-turn.
        // ChatRuntimeProvider clears it on chat_done / chat_error.
      })
      .addCase(addInferenceResponse.rejected, () => {
        // Do NOT clear activeThreadId here — ChatRuntimeProvider clears it on
        // chat_done / chat_error. Clearing on every rejected segment append
        // would re-enable the composer while the turn is still in-flight.
      })
      .addCase(persistMessageFeedback.fulfilled, (state, action) => {
        appendMessageToCache(state, action.payload.threadId, action.payload.message, true);
      })
      .addCase(deleteThread.fulfilled, (state, action) => {
        delete state.messagesByThreadId[action.payload.threadId];
        if (state.channelActivityByThreadId) {
          delete state.channelActivityByThreadId[action.payload.threadId];
        }
      })
      .addCase(updateThreadTitle.fulfilled, (state, action) => {
        const idx = state.threads.findIndex(t => t.id === action.payload.id);
        if (idx >= 0) {
          state.threads[idx] = action.payload;
        }
      })
      .addCase(updateThreadWorkingDir.fulfilled, (state, action) => {
        const idx = state.threads.findIndex(t => t.id === action.payload.id);
        if (idx >= 0) {
          state.threads[idx] = action.payload;
        }
      })
      .addCase(resetUserScopedState, () => initialState);
  },
});

export const {
  applyChannelEvent,
  clearCreateThreadError,
  setSelectedThread,
  invalidateThreadSelection,
  clearSelectedThread,
  setActiveThread,
  markThreadInferenceActive,
  clearThreadInferenceActive,
  clearStaleThread,
  clearAllThreads,
  resetThreadCachesPreservingSelection,
  setWelcomeThreadId,
  truncateMessagesFrom,
  removeMessagesById,
} = threadSlice.actions;

export default threadSlice.reducer;
