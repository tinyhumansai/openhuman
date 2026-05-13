import { createAsyncThunk, createSlice, type PayloadAction } from '@reduxjs/toolkit';

import { threadApi } from '../services/api/threadApi';
import { isThreadNotFoundCoreRpcError } from '../services/coreRpcClient';
import type { Thread, ThreadMessage } from '../types/thread';
import { IS_DEV } from '../utils/config';
import { resetUserScopedState } from './resetActions';

export const THREAD_NOT_FOUND_MESSAGE = 'This thread is no longer available.';

interface ThreadState {
  threads: Thread[];
  selectedThreadId: string | null;
  activeThreadId: string | null;
  // [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough
  // /**
  //  * Thread created by `OnboardingLayout` to host the proactive welcome
  //  * conversation. Tracked so we can delete it once the welcome agent
  //  * calls `complete_onboarding` and `chat_onboarding_completed` flips —
  //  * the welcome thread is transient onboarding chat, not history we
  //  * want to clutter the user's thread list with.
  //  */
  // welcomeThreadId: string | null;
  /** @deprecated [#1123] — welcome-agent replaced by Joyride walkthrough; kept for TS compat */
  welcomeThreadId: string | null;
  messagesByThreadId: Record<string, ThreadMessage[]>;
  messages: ThreadMessage[];
  isLoadingThreads: boolean;
  isLoadingMessages: boolean;
  messagesError: string | null;
}

const initialState: ThreadState = {
  threads: [],
  selectedThreadId: null,
  activeThreadId: null,
  welcomeThreadId: null,
  messagesByThreadId: {},
  messages: [],
  isLoadingThreads: false,
  isLoadingMessages: false,
  messagesError: null,
};

function appendMessageToCache(
  state: ThreadState,
  threadId: string,
  message: ThreadMessage,
  replaceExisting = false
) {
  const existing = state.messagesByThreadId[threadId] ?? [];
  const next = replaceExisting
    ? existing.map(e => (e.id === message.id ? message : e))
    : [...existing, message];
  state.messagesByThreadId[threadId] = next;
  if (threadId === state.selectedThreadId) {
    state.messages = replaceExisting
      ? state.messages.map(e => (e.id === message.id ? message : e))
      : [...state.messages, message];
  }
}

// ── Async thunks (thin RPC wrappers) ──────────────────────────────

export const loadThreads = createAsyncThunk(
  'thread/loadThreads',
  async (_, { rejectWithValue }) => {
    try {
      return await threadApi.getThreads();
    } catch (error) {
      return rejectWithValue(error instanceof Error ? error.message : 'Failed to load threads');
    }
  }
);

export const createNewThread = createAsyncThunk(
  'thread/createNewThread',
  async (labels: string[] | undefined, { dispatch, rejectWithValue }) => {
    try {
      const thread = await threadApi.createNewThread(labels);
      await dispatch(loadThreads()).unwrap();
      return thread;
    } catch (error) {
      return rejectWithValue(error instanceof Error ? error.message : 'Failed to create thread');
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
  async (threadId: string, { rejectWithValue }) => {
    try {
      const response = await threadApi.getThreadMessages(threadId);
      return { threadId, messages: response.messages };
    } catch (error) {
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
  async (payload: { threadId: string; message: ThreadMessage }, { dispatch, rejectWithValue }) => {
    try {
      const persisted = await threadApi.appendMessage(payload.threadId, payload.message);
      if (payload.message.sender === 'user' && payload.message.content.trim()) {
        try {
          await dispatch(generateThreadTitleIfNeeded({ threadId: payload.threadId })).unwrap();
        } catch (error) {
          if (IS_DEV) {
            console.debug('[threadSlice] addMessageLocal title refresh failed', {
              threadId: payload.threadId,
              error,
            });
          }
        }
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
    const targetThreadId = payload.threadId ?? state.thread.activeThreadId;
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

export const persistReaction = createAsyncThunk(
  'thread/persistReaction',
  async (
    payload: { threadId: string; messageId: string; emoji: string },
    { getState, rejectWithValue }
  ) => {
    const state = getState() as { thread: ThreadState };
    const stored = state.thread.messagesByThreadId[payload.threadId] ?? [];
    const message = stored.find(e => e.id === payload.messageId);
    if (!message) return rejectWithValue('Message not found');

    const prev = (message.extraMetadata['myReactions'] as string[] | undefined) ?? [];
    const idx = prev.indexOf(payload.emoji);
    const next = idx >= 0 ? prev.filter(e => e !== payload.emoji) : [...prev, payload.emoji];
    const extraMetadata = { ...message.extraMetadata, myReactions: next };

    try {
      const persisted = await threadApi.updateMessage(
        payload.threadId,
        payload.messageId,
        extraMetadata
      );
      return { threadId: payload.threadId, message: persisted };
    } catch (error) {
      return rejectWithValue(error instanceof Error ? error.message : 'Failed to save reaction');
    }
  }
);

export const updateThreadLabels = createAsyncThunk(
  'thread/updateThreadLabels',
  async (payload: { threadId: string; labels: string[] }, { dispatch, rejectWithValue }) => {
    try {
      const thread = await threadApi.updateLabels(payload.threadId, payload.labels);
      await dispatch(loadThreads()).unwrap();
      return thread;
    } catch (error) {
      return rejectWithValue(
        error instanceof Error ? error.message : 'Failed to update thread labels'
      );
    }
  }
);

export const purgeThreads = createAsyncThunk(
  'thread/purgeThreads',
  async (_, { dispatch, rejectWithValue }) => {
    try {
      const result = await threadApi.purge();
      dispatch(clearAllThreads());
      return result;
    } catch (error) {
      return rejectWithValue(error instanceof Error ? error.message : 'Failed to purge threads');
    }
  }
);

// ── Slice ─────────────────────────────────────────────────────────

const threadSlice = createSlice({
  name: 'thread',
  initialState,
  reducers: {
    setSelectedThread: (state, action: { payload: string }) => {
      state.selectedThreadId = action.payload;
      state.messages = state.messagesByThreadId[action.payload] ?? [];
      state.messagesError = null;
    },
    clearSelectedThread: state => {
      state.selectedThreadId = null;
      state.messages = [];
      state.messagesError = null;
    },
    setActiveThread: (state, action: { payload: string | null }) => {
      state.activeThreadId = action.payload;
    },
    clearStaleThread: (state, action: PayloadAction<string>) => {
      const threadId = action.payload;
      state.threads = state.threads.filter(thread => thread.id !== threadId);
      delete state.messagesByThreadId[threadId];
      if (state.selectedThreadId === threadId) {
        state.selectedThreadId = null;
        state.messages = [];
        state.messagesError = null;
      }
      if (state.activeThreadId === threadId) {
        state.activeThreadId = null;
      }
      if (state.welcomeThreadId === threadId) {
        state.welcomeThreadId = null;
      }
    },
    clearAllThreads: state => {
      state.threads = [];
      state.messagesByThreadId = {};
      state.selectedThreadId = null;
      state.messages = [];
      state.activeThreadId = null;
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
      state.messages = [];
      state.activeThreadId = null;
      state.welcomeThreadId = null;
    },
    // [#1123] True no-op — welcome-agent onboarding replaced by Joyride walkthrough.
    // Kept to avoid breaking existing imports; state.welcomeThreadId is never
    // mutated because the welcome-agent flow no longer runs.
    setWelcomeThreadId: () => {
      // intentional no-op
    },
  },
  extraReducers: builder => {
    builder
      .addCase(loadThreads.pending, state => {
        state.isLoadingThreads = true;
      })
      .addCase(loadThreads.fulfilled, (state, action) => {
        state.isLoadingThreads = false;
        state.threads = action.payload.threads;
        const liveThreadIds = new Set(action.payload.threads.map(thread => thread.id));
        if (state.selectedThreadId && !liveThreadIds.has(state.selectedThreadId)) {
          state.selectedThreadId = null;
          state.messages = [];
          state.messagesError = null;
        }
        if (state.activeThreadId && !liveThreadIds.has(state.activeThreadId)) {
          state.activeThreadId = null;
        }
      })
      .addCase(loadThreads.rejected, state => {
        state.isLoadingThreads = false;
      })
      .addCase(loadThreadMessages.pending, state => {
        state.isLoadingMessages = true;
        state.messagesError = null;
      })
      .addCase(loadThreadMessages.fulfilled, (state, action) => {
        state.isLoadingMessages = false;
        state.messagesByThreadId[action.payload.threadId] = action.payload.messages;
        if (action.payload.threadId === state.selectedThreadId) {
          state.messages = action.payload.messages;
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
      .addCase(persistReaction.fulfilled, (state, action) => {
        appendMessageToCache(state, action.payload.threadId, action.payload.message, true);
      })
      .addCase(deleteThread.fulfilled, (state, action) => {
        delete state.messagesByThreadId[action.payload.threadId];
      })
      .addCase(resetUserScopedState, () => initialState);
  },
});

export const {
  setSelectedThread,
  clearSelectedThread,
  setActiveThread,
  clearStaleThread,
  clearAllThreads,
  resetThreadCachesPreservingSelection,
  setWelcomeThreadId,
} = threadSlice.actions;

export default threadSlice.reducer;
