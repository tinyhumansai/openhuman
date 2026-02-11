import { createAsyncThunk, createSlice } from '@reduxjs/toolkit';

import { threadApi } from '../services/api/threadApi';
import type { Thread, ThreadMessage } from '../types/thread';

interface ThreadState {
  threads: Thread[];
  isLoading: boolean;
  error: string | null;
  selectedThreadId: string | null;
  messages: ThreadMessage[];
  isLoadingMessages: boolean;
  messagesError: string | null;
  createStatus: 'idle' | 'loading' | 'success' | 'error';
  purgeStatus: 'idle' | 'loading' | 'success' | 'error';
  sendStatus: 'idle' | 'loading' | 'success' | 'error';
  sendError: string | null;
}

const initialState: ThreadState = {
  threads: [],
  isLoading: false,
  error: null,
  selectedThreadId: null,
  messages: [],
  isLoadingMessages: false,
  messagesError: null,
  createStatus: 'idle',
  purgeStatus: 'idle',
  sendStatus: 'idle',
  sendError: null,
};

export const fetchThreads = createAsyncThunk(
  'thread/fetchThreads',
  async (_, { rejectWithValue }) => {
    try {
      const data = await threadApi.getThreads();
      return data.threads;
    } catch (error) {
      const msg =
        error && typeof error === 'object' && 'error' in error
          ? String(error.error)
          : 'Failed to fetch threads';
      return rejectWithValue(msg);
    }
  }
);

export const fetchThreadMessages = createAsyncThunk(
  'thread/fetchThreadMessages',
  async (threadId: string, { rejectWithValue }) => {
    try {
      const data = await threadApi.getThreadMessages(threadId);
      return data.messages;
    } catch (error) {
      const msg =
        error && typeof error === 'object' && 'error' in error
          ? String(error.error)
          : 'Failed to fetch messages';
      return rejectWithValue(msg);
    }
  }
);

export const createThread = createAsyncThunk(
  'thread/createThread',
  async (chatId: number | undefined, { dispatch, rejectWithValue }) => {
    try {
      const data = await threadApi.createThread(chatId);
      dispatch(fetchThreads());
      return data;
    } catch (error) {
      const msg =
        error && typeof error === 'object' && 'error' in error
          ? String(error.error)
          : 'Failed to create thread';
      return rejectWithValue(msg);
    }
  }
);

export const purgeThreads = createAsyncThunk(
  'thread/purgeThreads',
  async (_, { dispatch, rejectWithValue }) => {
    try {
      const data = await threadApi.purge({
        messages: false,
        agentThreads: true,
        deleteEverything: true,
      });
      dispatch(fetchThreads());
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
    try {
      const data = await threadApi.sendMessage(message, threadId);
      // Re-fetch messages to get the stored user message + agent response
      dispatch(fetchThreadMessages(threadId));
      // Re-fetch threads to update lastMessageAt / messageCount in the list
      dispatch(fetchThreads());
      return data;
    } catch (error) {
      const msg =
        error && typeof error === 'object' && 'error' in error
          ? String(error.error)
          : 'Failed to send message';
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
      state.messages = [];
      state.messagesError = null;
    },
    clearSelectedThread: state => {
      state.selectedThreadId = null;
      state.messages = [];
      state.messagesError = null;
    },
    clearCreateStatus: state => {
      state.createStatus = 'idle';
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
    clearSendError: state => {
      state.sendError = null;
    },
  },
  extraReducers: builder => {
    builder
      // fetchThreads
      .addCase(fetchThreads.pending, state => {
        state.isLoading = true;
        state.error = null;
      })
      .addCase(fetchThreads.fulfilled, (state, action) => {
        state.isLoading = false;
        state.threads = action.payload;
      })
      .addCase(fetchThreads.rejected, (state, action) => {
        state.isLoading = false;
        state.error = action.payload as string;
      })
      // fetchThreadMessages
      .addCase(fetchThreadMessages.pending, state => {
        state.isLoadingMessages = true;
        state.messagesError = null;
      })
      .addCase(fetchThreadMessages.fulfilled, (state, action) => {
        state.isLoadingMessages = false;
        state.messages = action.payload;
      })
      .addCase(fetchThreadMessages.rejected, (state, action) => {
        state.isLoadingMessages = false;
        state.messagesError = action.payload as string;
      })
      // createThread
      .addCase(createThread.pending, state => {
        state.createStatus = 'loading';
      })
      .addCase(createThread.fulfilled, (state, action) => {
        state.createStatus = 'success';
        state.selectedThreadId = action.payload.id;
        state.messages = [];
        state.messagesError = null;
      })
      .addCase(createThread.rejected, state => {
        state.createStatus = 'error';
      })
      // purgeThreads
      .addCase(purgeThreads.pending, state => {
        state.purgeStatus = 'loading';
      })
      .addCase(purgeThreads.fulfilled, state => {
        state.purgeStatus = 'success';
        state.selectedThreadId = null;
        state.messages = [];
      })
      .addCase(purgeThreads.rejected, state => {
        state.purgeStatus = 'error';
      })
      // sendMessage
      .addCase(sendMessage.pending, state => {
        state.sendStatus = 'loading';
        state.sendError = null;
      })
      .addCase(sendMessage.fulfilled, state => {
        state.sendStatus = 'success';
      })
      .addCase(sendMessage.rejected, (state, action) => {
        state.sendStatus = 'error';
        state.sendError = action.payload as string;
      });
  },
});

export const {
  setSelectedThread,
  clearSelectedThread,
  clearCreateStatus,
  clearPurgeStatus,
  addOptimisticMessage,
  clearSendError,
} = threadSlice.actions;
export default threadSlice.reducer;
