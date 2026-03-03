import { createAsyncThunk, createSlice } from '@reduxjs/toolkit';

import { threadApi } from '../services/api/threadApi';
import type { Thread, ThreadMessage } from '../types/thread';

interface ThreadState {
  // Existing local data (will be persisted)
  threads: Thread[];
  selectedThreadId: string | null;
  panelWidth: number;
  lastViewedAt: Record<string, number>;

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
    { dispatch, getState, rejectWithValue }
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

      // 2. Send to API (existing logic)
      const data = await threadApi.sendMessage(message, threadId);

      // 3. For now, we'll handle AI response via the existing inference API
      // The AI response will be added separately via addInferenceResponse

      return data;
    } catch (error) {
      // Remove optimistic user message on failure
      const state = (getState() as { thread: ThreadState }).thread;
      const messages = state.messagesByThreadId[threadId] || [];
      const filteredMessages = messages.filter(m => m.id !== userMessage.id);
      dispatch(updateMessagesForThread({ threadId, messages: filteredMessages }));

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
  async (conversationId: string | undefined, { rejectWithValue }) => {
    try {
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
    addInferenceResponse: (state, action: { payload: { content: string } }) => {
      const aiMessage: ThreadMessage = {
        id: `inference-${Date.now()}`,
        content: action.payload.content,
        type: 'text',
        extraMetadata: {},
        sender: 'agent',
        createdAt: new Date().toISOString(),
      };

      // Add to current messages view
      state.messages.push(aiMessage);

      // Also add to persistent storage
      if (state.selectedThreadId) {
        if (!state.messagesByThreadId[state.selectedThreadId]) {
          state.messagesByThreadId[state.selectedThreadId] = [];
        }

        // CRITICAL FIX: Ensure the preceding user message is also persisted
        // Find the last user message that might not be in persistent storage yet
        const lastUserMessage = state.messages.filter(m => m.sender === 'user').pop();

        if (lastUserMessage) {
          const persistedMessages = state.messagesByThreadId[state.selectedThreadId];
          const userMessageExists = persistedMessages.some(m => m.id === lastUserMessage.id);

          // If user message isn't persisted yet, add it first
          if (!userMessageExists) {
            persistedMessages.push(lastUserMessage);
          }
        }

        // Now add the AI response
        state.messagesByThreadId[state.selectedThreadId].push(aiMessage);

        // Update thread metadata
        const thread = state.threads.find(t => t.id === state.selectedThreadId);
        if (thread) {
          thread.messageCount = state.messagesByThreadId[state.selectedThreadId].length;
          thread.lastMessageAt = aiMessage.createdAt;
        }
      }
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
      })
      // sendMessage
      .addCase(sendMessage.pending, state => {
        state.sendStatus = 'loading';
        state.sendError = null;
      })
      .addCase(sendMessage.fulfilled, state => {
        state.sendStatus = 'success';
        state.suggestedQuestions = [];
        state.suggestError = null;
      })
      .addCase(sendMessage.rejected, (state, action) => {
        state.sendStatus = 'error';
        state.sendError = action.payload as string;
        // Remove optimistic messages so the user doesn't see phantom messages
        state.messages = state.messages.filter(m => !m.id.startsWith('optimistic-'));
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
} = threadSlice.actions;
export default threadSlice.reducer;
