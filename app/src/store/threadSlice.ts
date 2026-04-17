import { createAsyncThunk, createSlice } from '@reduxjs/toolkit';

import { threadApi } from '../services/api/threadApi';
import type { Thread, ThreadMessage } from '../types/thread';
import { isTauri, openhumanLocalAiSuggestQuestions } from '../utils/tauriCommands';

interface ThreadState {
  threads: Thread[];
  selectedThreadId: string | null;
  activeThreadId: string | null;
  messagesByThreadId: Record<string, ThreadMessage[]>;
  messages: ThreadMessage[];
  isLoadingThreads: boolean;
  isLoadingMessages: boolean;
  messagesError: string | null;
  suggestedQuestions: Array<{ text: string; confidence: number }>;
  isLoadingSuggestions: boolean;
}

const initialState: ThreadState = {
  threads: [],
  selectedThreadId: null,
  activeThreadId: null,
  messagesByThreadId: {},
  messages: [],
  isLoadingThreads: false,
  isLoadingMessages: false,
  messagesError: null,
  suggestedQuestions: [],
  isLoadingSuggestions: false,
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
  async (_, { dispatch, rejectWithValue }) => {
    try {
      const thread = await threadApi.createNewThread();
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

export const addMessageLocal = createAsyncThunk(
  'thread/addMessageLocal',
  async (payload: { threadId: string; message: ThreadMessage }, { rejectWithValue }) => {
    try {
      const persisted = await threadApi.appendMessage(payload.threadId, payload.message);
      return { threadId: payload.threadId, message: persisted };
    } catch (error) {
      return rejectWithValue(error instanceof Error ? error.message : 'Failed to save message');
    }
  }
);

export const addInferenceResponse = createAsyncThunk(
  'thread/addInferenceResponse',
  async (
    payload: { content: string; threadId?: string; messageId?: string; type?: string },
    { getState, rejectWithValue }
  ) => {
    const state = getState() as { thread: ThreadState };
    const targetThreadId = payload.threadId ?? state.thread.activeThreadId;
    if (!targetThreadId) return rejectWithValue('No target thread');

    const message: ThreadMessage = {
      id: payload.messageId ?? `inference-${Date.now()}-${Math.random()}`,
      content: payload.content,
      type: payload.type ?? 'text',
      extraMetadata: {},
      sender: 'agent',
      createdAt: new Date().toISOString(),
    };

    try {
      const persisted = await threadApi.appendMessage(targetThreadId, message);
      return { threadId: targetThreadId, message: persisted };
    } catch (error) {
      return rejectWithValue(error instanceof Error ? error.message : 'Failed to save response');
    }
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

export const fetchSuggestedQuestions = createAsyncThunk(
  'thread/fetchSuggestedQuestions',
  async (conversationId: string | undefined, { getState, rejectWithValue }) => {
    try {
      const state = getState() as { thread: ThreadState };
      const tid = conversationId ?? state.thread.selectedThreadId ?? undefined;
      const msgs = tid ? (state.thread.messagesByThreadId[tid] ?? []) : [];

      if (isTauri()) {
        const lines = msgs
          .slice(-24)
          .map(m => `${m.sender === 'user' ? 'User' : 'Assistant'}: ${m.content}`);
        const local = await openhumanLocalAiSuggestQuestions(undefined, lines);
        return local.result;
      }
      return [];
    } catch (error) {
      return rejectWithValue(
        error instanceof Error ? error.message : 'Failed to load suggested questions'
      );
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
      state.suggestedQuestions = [];
    },
    clearSelectedThread: state => {
      state.selectedThreadId = null;
      state.messages = [];
      state.messagesError = null;
      state.suggestedQuestions = [];
    },
    setActiveThread: (state, action: { payload: string | null }) => {
      state.activeThreadId = action.payload;
    },
    clearAllThreads: state => {
      state.threads = [];
      state.messagesByThreadId = {};
      state.selectedThreadId = null;
      state.messages = [];
      state.activeThreadId = null;
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
      .addCase(fetchSuggestedQuestions.pending, state => {
        state.isLoadingSuggestions = true;
      })
      .addCase(fetchSuggestedQuestions.fulfilled, (state, action) => {
        state.isLoadingSuggestions = false;
        state.suggestedQuestions = action.payload;
      })
      .addCase(fetchSuggestedQuestions.rejected, state => {
        state.isLoadingSuggestions = false;
        state.suggestedQuestions = [];
      });
  },
});

export const { setSelectedThread, clearSelectedThread, setActiveThread, clearAllThreads } =
  threadSlice.actions;

export default threadSlice.reducer;
