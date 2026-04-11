import { createAsyncThunk, createSlice } from '@reduxjs/toolkit';

import { threadApi } from '../services/api/threadApi';
import type { Thread, ThreadMessage } from '../types/thread';
import { isTauri, openhumanLocalAiSuggestQuestions } from '../utils/tauriCommands';

interface ThreadState {
  threads: Thread[];
  selectedThreadId: string | null;
  panelWidth: number;
  lastViewedAt: Record<string, number>;
  activeThreadId: string | null;
  messagesByThreadId: Record<string, ThreadMessage[]>;
  messages: ThreadMessage[];
  isLoadingThreads: boolean;
  isLoadingMessages: boolean;
  messagesError: string | null;
  sendStatus: 'idle' | 'loading' | 'success' | 'error';
  sendError: string | null;
  deleteStatus: 'idle' | 'loading' | 'success' | 'error';
  purgeStatus: 'idle' | 'loading' | 'success' | 'error';
  suggestedQuestions: Array<{ text: string; confidence: number }>;
  isLoadingSuggestions: boolean;
  suggestError: string | null;
}

const initialState: ThreadState = {
  threads: [],
  selectedThreadId: null,
  panelWidth: 320,
  lastViewedAt: {},
  activeThreadId: null,
  messagesByThreadId: {},
  messages: [],
  isLoadingThreads: false,
  isLoadingMessages: false,
  messagesError: null,
  sendStatus: 'idle',
  sendError: null,
  deleteStatus: 'idle',
  purgeStatus: 'idle',
  suggestedQuestions: [],
  isLoadingSuggestions: false,
  suggestError: null,
};

function appendMessageToCache(
  state: ThreadState,
  threadId: string,
  message: ThreadMessage,
  replaceExisting = false
) {
  const existing = state.messagesByThreadId[threadId] ?? [];
  const nextStored = replaceExisting
    ? existing.map(entry => (entry.id === message.id ? message : entry))
    : [...existing, message];
  state.messagesByThreadId[threadId] = nextStored;

  if (threadId === state.selectedThreadId) {
    state.messages = replaceExisting
      ? state.messages.map(entry => (entry.id === message.id ? message : entry))
      : [...state.messages, message];
  }

  const thread = state.threads.find(entry => entry.id === threadId);
  if (thread) {
    thread.messageCount = nextStored.length;
    thread.lastMessageAt =
      nextStored.length > 0 ? nextStored[nextStored.length - 1].createdAt : thread.createdAt;
  }
}

function replaceMessagesForThread(state: ThreadState, threadId: string, messages: ThreadMessage[]) {
  state.messagesByThreadId[threadId] = messages;
  if (threadId === state.selectedThreadId) {
    state.messages = messages;
  }
  const thread = state.threads.find(entry => entry.id === threadId);
  if (thread) {
    thread.messageCount = messages.length;
    thread.lastMessageAt =
      messages.length > 0 ? messages[messages.length - 1].createdAt : thread.createdAt;
  }
}

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

export const createThreadLocal = createAsyncThunk(
  'thread/createThreadLocal',
  async (
    payload: { id: string; title: string; createdAt: string },
    { dispatch, rejectWithValue }
  ) => {
    try {
      const created = await threadApi.createThread(payload);
      await dispatch(loadThreads()).unwrap();
      return created;
    } catch (error) {
      return rejectWithValue(error instanceof Error ? error.message : 'Failed to create thread');
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
    if (!targetThreadId) {
      return rejectWithValue('No target thread for inference response');
    }

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
    const message = stored.find(entry => entry.id === payload.messageId);
    if (!message) {
      return rejectWithValue('Message not found for reaction update');
    }

    const prev = (message.extraMetadata['myReactions'] as string[] | undefined) ?? [];
    const idx = prev.indexOf(payload.emoji);
    const next =
      idx >= 0 ? prev.filter(entry => entry !== payload.emoji) : [...prev, payload.emoji];
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
      const selectedThreadId = conversationId ?? state.thread.selectedThreadId ?? undefined;
      const threadMessages = selectedThreadId
        ? (state.thread.messagesByThreadId[selectedThreadId] ?? [])
        : [];

      if (isTauri()) {
        const lines = threadMessages
          .slice(-24)
          .map(msg => `${msg.sender === 'user' ? 'User' : 'Assistant'}: ${msg.content}`);
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

const threadSlice = createSlice({
  name: 'thread',
  initialState,
  reducers: {
    setSelectedThread: (state, action: { payload: string }) => {
      state.selectedThreadId = action.payload;
      state.messages = state.messagesByThreadId[action.payload] ?? [];
      state.messagesError = null;
      state.suggestedQuestions = [];
      state.suggestError = null;
    },
    clearSelectedThread: state => {
      state.selectedThreadId = null;
      state.messages = [];
      state.messagesError = null;
      state.suggestedQuestions = [];
      state.suggestError = null;
    },
    clearSuggestedQuestions: state => {
      state.suggestedQuestions = [];
      state.suggestError = null;
    },
    clearDeleteStatus: state => {
      state.deleteStatus = 'idle';
    },
    clearPurgeStatus: state => {
      state.purgeStatus = 'idle';
    },
    clearSendError: state => {
      state.sendError = null;
    },
    setPanelWidth: (state, action: { payload: number }) => {
      state.panelWidth = action.payload;
    },
    setLastViewed: (state, action: { payload: string }) => {
      state.lastViewedAt[action.payload] = Date.now();
    },
    clearAllThreads: state => {
      state.threads = [];
      state.messagesByThreadId = {};
      state.selectedThreadId = null;
      state.messages = [];
      state.lastViewedAt = {};
      state.activeThreadId = null;
    },
    setActiveThread: (state, action: { payload: string | null }) => {
      state.activeThreadId = action.payload;
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
      .addCase(createThreadLocal.pending, state => {
        state.isLoadingThreads = true;
      })
      .addCase(createThreadLocal.fulfilled, state => {
        state.isLoadingThreads = false;
      })
      .addCase(createThreadLocal.rejected, (state, action) => {
        state.isLoadingThreads = false;
        state.messagesError = action.payload as string;
      })
      .addCase(loadThreadMessages.pending, state => {
        state.isLoadingMessages = true;
        state.messagesError = null;
      })
      .addCase(loadThreadMessages.fulfilled, (state, action) => {
        state.isLoadingMessages = false;
        replaceMessagesForThread(state, action.payload.threadId, action.payload.messages);
      })
      .addCase(loadThreadMessages.rejected, (state, action) => {
        state.isLoadingMessages = false;
        state.messagesError = action.payload as string;
      })
      .addCase(addMessageLocal.pending, state => {
        state.sendStatus = 'loading';
        state.sendError = null;
      })
      .addCase(addMessageLocal.fulfilled, (state, action) => {
        state.sendStatus = 'success';
        appendMessageToCache(state, action.payload.threadId, action.payload.message);
      })
      .addCase(addMessageLocal.rejected, (state, action) => {
        state.sendStatus = 'error';
        state.sendError = action.payload as string;
      })
      .addCase(addInferenceResponse.fulfilled, (state, action) => {
        appendMessageToCache(state, action.payload.threadId, action.payload.message);
        state.activeThreadId = null;
      })
      .addCase(addInferenceResponse.rejected, (state, action) => {
        state.sendError = action.payload as string;
        state.activeThreadId = null;
      })
      .addCase(persistReaction.fulfilled, (state, action) => {
        appendMessageToCache(state, action.payload.threadId, action.payload.message, true);
      })
      .addCase(purgeThreads.pending, state => {
        state.purgeStatus = 'loading';
      })
      .addCase(purgeThreads.fulfilled, state => {
        state.purgeStatus = 'success';
      })
      .addCase(purgeThreads.rejected, state => {
        state.purgeStatus = 'error';
      })
      .addCase(fetchSuggestedQuestions.pending, state => {
        state.isLoadingSuggestions = true;
        state.suggestError = null;
      })
      .addCase(fetchSuggestedQuestions.fulfilled, (state, action) => {
        state.isLoadingSuggestions = false;
        state.suggestedQuestions = action.payload;
      })
      .addCase(fetchSuggestedQuestions.rejected, (state, action) => {
        state.isLoadingSuggestions = false;
        state.suggestError = action.payload as string;
        state.suggestedQuestions = [];
      });
  },
});

export const {
  setSelectedThread,
  clearSelectedThread,
  clearDeleteStatus,
  clearPurgeStatus,
  clearSendError,
  clearSuggestedQuestions,
  setPanelWidth,
  setLastViewed,
  clearAllThreads,
  setActiveThread,
} = threadSlice.actions;

export default threadSlice.reducer;
