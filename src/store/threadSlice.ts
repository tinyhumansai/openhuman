import { createAsyncThunk, createSlice } from '@reduxjs/toolkit';

import { threadApi } from '../services/api/threadApi';
import { isTauri, openhumanLocalAiSuggestQuestions } from '../utils/tauriCommands';
import type { Thread, ThreadMessage } from '../types/thread';

interface ThreadState {
  // Existing local data (will be persisted)
  threads: Thread[];
  selectedThreadId: string | null;
  panelWidth: number;
  lastViewedAt: Record<string, number>;
  activeThreadId: string | null; // Track which thread is currently sending/receiving

  // NEW: Add efficient message storage
  messagesByThreadId: Record<string, ThreadMessage[]>;

  // Current messages view (not persisted)
  messages: ThreadMessage[];

  // Keep these API states (NOT persisted)
  isLoadingMessages: boolean; // For AI response waiting
  messagesError: string | null; // For send API errors
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

// Removed fetchThreads - threads are now managed locally

// Removed fetchThreadMessages - messages are now managed locally

// Removed createThread - using local thread creation

// Removed deleteThread - using local thread deletion

export const purgeThreads = createAsyncThunk(
  'thread/purgeThreads',
  async (_, { dispatch, rejectWithValue }) => {
    try {
      const data = await threadApi.purge({
        messages: false,
        agentThreads: true,
        deleteEverything: true,
      });
      // Clear local threads after successful purge
      dispatch({ type: 'thread/clearAllThreads' });
      return data;
    } catch (error) {
      const msg =
        error && typeof error === 'object' && 'error' in error
          ? String(error.error)
          : 'Failed to purge threads';
      return rejectWithValue(msg);
    }
  }
);

export const sendMessage = createAsyncThunk(
  'thread/sendMessage',
  async (
    { threadId, message }: { threadId: string; message: string },
    { dispatch, rejectWithValue }
  ) => {
    // 1. Add user message locally immediately (optimistic update)
    const userMessage: ThreadMessage = {
      id: `msg_${Date.now()}_${Math.random()}`,
      content: message,
      type: 'text',
      extraMetadata: {},
      sender: 'user',
      createdAt: new Date().toISOString(),
    };

    try {
      dispatch(addMessageLocal({ threadId, message: userMessage }));

      // 2. Send plain message - orchestration and context injection happen in Rust.
      const data = await threadApi.sendMessage(message, threadId);

      // 3. For now, we'll handle AI response via the existing inference API
      // The AI response will be added separately via addInferenceResponse

      return data;
    } catch (error) {
      // Add an error message as an agent response so the conversation flow continues
      dispatch(
        addInferenceResponse({ content: 'Something went wrong — please try again.', threadId })
      );

      const msg =
        error && typeof error === 'object' && 'error' in error
          ? String((error as { error: unknown }).error)
          : 'Failed to send message';
      return rejectWithValue(msg);
    }
  }
);

export const fetchSuggestedQuestions = createAsyncThunk(
  'thread/fetchSuggestedQuestions',
  async (conversationId: string | undefined, { getState, rejectWithValue }) => {
    try {
      if (isTauri()) {
        const state = getState() as {
          thread?: {
            selectedThreadId?: string | null;
            messagesByThreadId?: Record<string, ThreadMessage[]>;
          };
        };
        const selectedThreadId = conversationId ?? state.thread?.selectedThreadId ?? undefined;
        const threadMessages = selectedThreadId
          ? (state.thread?.messagesByThreadId?.[selectedThreadId] ?? [])
          : [];
        const lines = threadMessages
          .slice(-24)
          .map(msg => `${msg.sender === 'user' ? 'User' : 'Assistant'}: ${msg.content}`);
        const local = await openhumanLocalAiSuggestQuestions(undefined, lines);
        return local.result;
      }
      const data = await threadApi.getSuggestQuestions(conversationId);
      return data.suggestions;
    } catch (error) {
      const msg =
        error && typeof error === 'object' && 'error' in error
          ? String(error.error)
          : 'Failed to load suggestions';
      return rejectWithValue(msg);
    }
  }
);

const threadSlice = createSlice({
  name: 'thread',
  initialState,
  reducers: {
    setSelectedThread: (state, action: { payload: string }) => {
      state.selectedThreadId = action.payload;
      // Load messages from local storage instead of clearing
      state.messages = state.messagesByThreadId[action.payload] || [];
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
    addOptimisticMessage: (state, action: { payload: { content: string } }) => {
      state.messages.push({
        id: `optimistic-${Date.now()}`,
        content: action.payload.content,
        type: 'text',
        extraMetadata: {},
        sender: 'user',
        createdAt: new Date().toISOString(),
      });
    },
    addInferenceResponse: (state, action: { payload: { content: string; threadId?: string } }) => {
      const aiMessage: ThreadMessage = {
        id: `inference-${Date.now()}`,
        content: action.payload.content,
        type: 'text',
        extraMetadata: {},
        sender: 'agent',
        createdAt: new Date().toISOString(),
      };

      // Use provided threadId or fall back to activeThreadId to ensure response goes to correct thread
      const targetThreadId = action.payload.threadId || state.activeThreadId;

      if (targetThreadId) {
        // Ensure messagesByThreadId exists for this thread
        if (!state.messagesByThreadId[targetThreadId]) {
          state.messagesByThreadId[targetThreadId] = [];
        }

        // Add the AI response to persistent storage
        state.messagesByThreadId[targetThreadId].push(aiMessage);

        // Add to current messages view only if it's the currently selected thread
        if (targetThreadId === state.selectedThreadId) {
          state.messages.push(aiMessage);
        }

        // Update thread metadata
        const thread = state.threads.find(t => t.id === targetThreadId);
        if (thread) {
          thread.messageCount = state.messagesByThreadId[targetThreadId].length;
          thread.lastMessageAt = aiMessage.createdAt;
        }
      }

      // Clear active thread when response is received
      state.activeThreadId = null;
    },
    removeOptimisticMessages: state => {
      state.messages = state.messages.filter(m => !m.id.startsWith('optimistic-'));
    },
    clearSendError: state => {
      state.sendError = null;
    },
    setPanelWidth: (state, action: { payload: number }) => {
      state.panelWidth = action.payload;
    },
    setLastViewed: (state, action: { payload: string }) => {
      const ts = Date.now();
      state.lastViewedAt[action.payload] = ts;
    },
    // Local thread management
    createThreadLocal: (
      state,
      action: { payload: { id: string; title: string; createdAt: string } }
    ) => {
      const newThread: Thread = {
        id: action.payload.id,
        title: action.payload.title,
        chatId: null,
        isActive: true,
        messageCount: 0,
        lastMessageAt: action.payload.createdAt,
        createdAt: action.payload.createdAt,
      };
      state.threads.unshift(newThread);
      state.messagesByThreadId[action.payload.id] = [];
    },
    addMessageLocal: (state, action: { payload: { threadId: string; message: ThreadMessage } }) => {
      const { threadId, message } = action.payload;
      if (!state.messagesByThreadId[threadId]) {
        state.messagesByThreadId[threadId] = [];
      }
      state.messagesByThreadId[threadId].push(message);

      // Also update current messages view if this is the selected thread
      if (threadId === state.selectedThreadId) {
        state.messages.push(message);
      }

      // Update thread metadata
      const thread = state.threads.find(t => t.id === threadId);
      if (thread) {
        thread.messageCount = state.messagesByThreadId[threadId].length;
        thread.lastMessageAt = message.createdAt;
      }
    },
    deleteThreadLocal: (state, action: { payload: string }) => {
      const threadId = action.payload;
      state.threads = state.threads.filter(t => t.id !== threadId);
      delete state.messagesByThreadId[threadId];
      delete state.lastViewedAt[threadId];
      if (state.selectedThreadId === threadId) {
        state.selectedThreadId = null;
      }
    },
    updateMessagesForThread: (
      state,
      action: { payload: { threadId: string; messages: ThreadMessage[] } }
    ) => {
      const { threadId, messages } = action.payload;
      state.messagesByThreadId[threadId] = messages;

      // Update thread metadata
      const thread = state.threads.find(t => t.id === threadId);
      if (thread) {
        thread.messageCount = messages.length;
        thread.lastMessageAt =
          messages.length > 0 ? messages[messages.length - 1].createdAt : thread.createdAt;
      }
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
      // Removed fetchThreads and fetchThreadMessages cases
      // Removed createThread cases - using local thread creation
      // Removed deleteThread cases - using local thread deletion
      // purgeThreads
      .addCase(purgeThreads.pending, state => {
        state.purgeStatus = 'loading';
      })
      .addCase(purgeThreads.fulfilled, state => {
        state.purgeStatus = 'success';
        // clearAllThreads is dispatched from the thunk
      })
      .addCase(purgeThreads.rejected, state => {
        state.purgeStatus = 'error';
      })
      // clearAllThreads action
      .addCase('thread/clearAllThreads', state => {
        state.threads = [];
        state.messagesByThreadId = {};
        state.selectedThreadId = null;
        state.messages = [];
        state.lastViewedAt = {};
        state.activeThreadId = null;
      })
      // sendMessage
      .addCase(sendMessage.pending, (state, action) => {
        state.sendStatus = 'loading';
        state.sendError = null;
        // Set the active thread when message sending starts
        state.activeThreadId = action.meta.arg.threadId;
      })
      .addCase(sendMessage.fulfilled, state => {
        state.sendStatus = 'success';
        state.suggestedQuestions = [];
        state.suggestError = null;
        // Don't clear activeThreadId here - let addInferenceResponse handle it
      })
      .addCase(sendMessage.rejected, (state, action) => {
        state.sendStatus = 'error';
        state.sendError = action.payload as string;
        // Remove optimistic messages so the user doesn't see phantom messages
        state.messages = state.messages.filter(m => !m.id.startsWith('optimistic-'));
        // Clear active thread on error
        state.activeThreadId = null;
      })
      // fetchSuggestedQuestions
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
  addOptimisticMessage,
  addInferenceResponse,
  removeOptimisticMessages,
  clearSendError,
  clearSuggestedQuestions,
  setPanelWidth,
  setLastViewed,
  createThreadLocal,
  addMessageLocal,
  deleteThreadLocal,
  updateMessagesForThread,
  clearAllThreads,
  setActiveThread,
} = threadSlice.actions;
export default threadSlice.reducer;
