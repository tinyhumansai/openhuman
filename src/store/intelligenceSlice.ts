import { createAsyncThunk, createSlice, type PayloadAction } from '@reduxjs/toolkit';

import { intelligenceApi, type ConnectedTool } from '../services/intelligenceApi';
import type {
  ActionableItem,
  ActionableItemSource,
  ActionableItemStatus,
  ChatMessage,
} from '../types/intelligence';
import {
  transformBackendItemsToFrontend,
  transformBackendMessagesToFrontend,
} from '../utils/intelligenceTransforms';

/**
 * Chat session state for managing individual conversations
 */
export interface ChatSessionState {
  threadId: string;
  itemId: string;
  messages: ChatMessage[];
  isTyping: boolean;
  lastMessageId?: string;
  isConnected: boolean;
}

/**
 * Execution state for tracking task progress
 */
export interface ExecutionState {
  executionId: string;
  sessionId: string;
  itemId: string;
  status: 'idle' | 'starting' | 'running' | 'completed' | 'failed';
  progress: Array<{
    id: string;
    label: string;
    status: 'pending' | 'in_progress' | 'completed' | 'failed';
    timestamp?: Date;
  }>;
  result?: unknown;
  error?: string;
}

/**
 * Intelligence Redux state
 */
export interface IntelligenceState {
  // Actionable Items
  items: ActionableItem[];
  loading: boolean;
  error: string | null;
  lastUpdate: Date | null;

  // Chat Sessions
  activeSessions: Record<string, ChatSessionState>;
  currentChatSession: string | null;

  // Execution Management
  activeExecutions: Record<string, ExecutionState>;

  // UI State
  filters: {
    source: ActionableItemSource | 'all';
    priority: 'critical' | 'important' | 'normal' | 'all';
    search: string;
  };

  // System state
  initialized: boolean;
  connectionStatus: 'disconnected' | 'connecting' | 'connected' | 'error';
}

const initialState: IntelligenceState = {
  // Actionable Items
  items: [],
  loading: false,
  error: null,
  lastUpdate: null,

  // Chat Sessions
  activeSessions: {},
  currentChatSession: null,

  // Execution Management
  activeExecutions: {},

  // UI State
  filters: { source: 'all', priority: 'all', search: '' },

  // System state
  initialized: false,
  connectionStatus: 'disconnected',
};

// Async thunks for API operations

/**
 * Fetch actionable items from backend
 */
export const fetchActionableItems = createAsyncThunk(
  'intelligence/fetchActionableItems',
  async (_, { rejectWithValue }) => {
    try {
      const backendItems = await intelligenceApi.getActionableItems();
      return transformBackendItemsToFrontend(backendItems);
    } catch (error) {
      return rejectWithValue(
        error && typeof error === 'object' && 'error' in error
          ? error.error
          : 'Failed to fetch actionable items'
      );
    }
  }
);

/**
 * Update item status
 */
export const updateItemStatus = createAsyncThunk(
  'intelligence/updateItemStatus',
  async (
    { itemId, status }: { itemId: string; status: ActionableItemStatus },
    { rejectWithValue }
  ) => {
    try {
      await intelligenceApi.updateItemStatus(itemId, status);
      return { itemId, status, updatedAt: new Date() };
    } catch (error) {
      return rejectWithValue(
        error && typeof error === 'object' && 'error' in error
          ? error.error
          : 'Failed to update item status'
      );
    }
  }
);

/**
 * Snooze item until specific time
 */
export const snoozeItem = createAsyncThunk(
  'intelligence/snoozeItem',
  async ({ itemId, snoozeUntil }: { itemId: string; snoozeUntil: Date }, { rejectWithValue }) => {
    try {
      await intelligenceApi.snoozeItem(itemId, snoozeUntil);
      return { itemId, snoozeUntil, updatedAt: new Date() };
    } catch (error) {
      return rejectWithValue(
        error && typeof error === 'object' && 'error' in error
          ? error.error
          : 'Failed to snooze item'
      );
    }
  }
);

/**
 * Get or create chat session
 */
export const createChatSession = createAsyncThunk(
  'intelligence/createChatSession',
  async ({ itemId }: { itemId: string }, { rejectWithValue }) => {
    try {
      const threadResponse = await intelligenceApi.getOrCreateThread(itemId);
      const messages = transformBackendMessagesToFrontend(threadResponse.messages);

      return { threadId: threadResponse.threadId, itemId, messages };
    } catch (error) {
      return rejectWithValue(
        error && typeof error === 'object' && 'error' in error
          ? error.error
          : 'Failed to create chat session'
      );
    }
  }
);

/**
 * Execute task with connected tools
 */
export const executeTask = createAsyncThunk(
  'intelligence/executeTask',
  async (
    { itemId, connectedTools }: { itemId: string; connectedTools: ConnectedTool[] },
    { rejectWithValue }
  ) => {
    try {
      const response = await intelligenceApi.executeTask(itemId, connectedTools);
      return {
        executionId: response.executionId,
        sessionId: response.sessionId,
        itemId,
        status: response.status,
      };
    } catch (error) {
      return rejectWithValue(
        error && typeof error === 'object' && 'error' in error
          ? error.error
          : 'Failed to execute task'
      );
    }
  }
);

/**
 * Intelligence slice
 */
export const intelligenceSlice = createSlice({
  name: 'intelligence',
  initialState,
  reducers: {
    // System actions
    setInitialized: (state, action: PayloadAction<boolean>) => {
      state.initialized = action.payload;
    },

    setConnectionStatus: (
      state,
      action: PayloadAction<'disconnected' | 'connecting' | 'connected' | 'error'>
    ) => {
      state.connectionStatus = action.payload;
    },

    // Items actions
    setItems: (state, action: PayloadAction<ActionableItem[]>) => {
      state.items = action.payload;
      state.lastUpdate = new Date();
    },

    addItem: (state, action: PayloadAction<ActionableItem>) => {
      state.items.unshift(action.payload);
      state.lastUpdate = new Date();
    },

    removeItem: (state, action: PayloadAction<string>) => {
      state.items = state.items.filter(item => item.id !== action.payload);
      state.lastUpdate = new Date();
    },

    // Filters actions
    setSourceFilter: (state, action: PayloadAction<ActionableItemSource | 'all'>) => {
      state.filters.source = action.payload;
    },

    setPriorityFilter: (
      state,
      action: PayloadAction<'critical' | 'important' | 'normal' | 'all'>
    ) => {
      state.filters.priority = action.payload;
    },

    setSearchFilter: (state, action: PayloadAction<string>) => {
      state.filters.search = action.payload;
    },

    // Chat session actions
    setChatSession: (
      state,
      action: PayloadAction<{ threadId: string; itemId: string; messages?: ChatMessage[] }>
    ) => {
      const { threadId, itemId, messages = [] } = action.payload;
      state.activeSessions[threadId] = {
        threadId,
        itemId,
        messages,
        isTyping: false,
        isConnected: true,
      };
      state.currentChatSession = threadId;
    },

    addMessage: (state, action: PayloadAction<{ threadId: string; message: ChatMessage }>) => {
      const { threadId, message } = action.payload;
      const session = state.activeSessions[threadId];
      if (session) {
        session.messages.push(message);
        session.lastMessageId = message.id;
      }
    },

    setTyping: (state, action: PayloadAction<{ threadId: string; isTyping: boolean }>) => {
      const { threadId, isTyping } = action.payload;
      const session = state.activeSessions[threadId];
      if (session) {
        session.isTyping = isTyping;
      }
    },

    closeChatSession: (state, action: PayloadAction<string>) => {
      const threadId = action.payload;
      delete state.activeSessions[threadId];
      if (state.currentChatSession === threadId) {
        state.currentChatSession = null;
      }
    },

    setCurrentChatSession: (state, action: PayloadAction<string | null>) => {
      state.currentChatSession = action.payload;
    },

    // Execution actions
    setExecution: (
      state,
      action: PayloadAction<{ executionId: string; execution: ExecutionState }>
    ) => {
      const { executionId, execution } = action.payload;
      state.activeExecutions[executionId] = execution;
    },

    updateExecutionProgress: (
      state,
      action: PayloadAction<{ executionId: string; progress: ExecutionState['progress'] }>
    ) => {
      const { executionId, progress } = action.payload;
      const execution = state.activeExecutions[executionId];
      if (execution) {
        execution.progress = progress;
        execution.status = 'running';
      }
    },

    setExecutionResult: (
      state,
      action: PayloadAction<{
        executionId: string;
        result: unknown;
        status: 'completed' | 'failed';
        error?: string;
      }>
    ) => {
      const { executionId, result, status, error } = action.payload;
      const execution = state.activeExecutions[executionId];
      if (execution) {
        execution.result = result;
        execution.status = status;
        execution.error = error;
      }
    },

    clearExecution: (state, action: PayloadAction<string>) => {
      const executionId = action.payload;
      delete state.activeExecutions[executionId];
    },

    // Error handling
    clearError: state => {
      state.error = null;
    },
  },

  extraReducers: builder => {
    // Fetch actionable items
    builder
      .addCase(fetchActionableItems.pending, state => {
        state.loading = true;
        state.error = null;
      })
      .addCase(fetchActionableItems.fulfilled, (state, action) => {
        state.loading = false;
        state.items = action.payload;
        state.lastUpdate = new Date();
        state.error = null;
      })
      .addCase(fetchActionableItems.rejected, (state, action) => {
        state.loading = false;
        state.error = action.payload as string;
      });

    // Update item status
    builder
      .addCase(updateItemStatus.fulfilled, (state, action) => {
        const { itemId, status, updatedAt } = action.payload;
        const item = state.items.find(item => item.id === itemId);
        if (item) {
          item.status = status;
          item.updatedAt = updatedAt;

          // Set completion/dismissal timestamps
          if (status === 'completed') {
            item.completedAt = updatedAt;
          } else if (status === 'dismissed') {
            item.dismissedAt = updatedAt;
          }
        }
        state.lastUpdate = new Date();
      })
      .addCase(updateItemStatus.rejected, (state, action) => {
        state.error = action.payload as string;
      });

    // Snooze item
    builder
      .addCase(snoozeItem.fulfilled, (state, action) => {
        const { itemId, snoozeUntil, updatedAt } = action.payload;
        const item = state.items.find(item => item.id === itemId);
        if (item) {
          item.status = 'snoozed';
          item.snoozeUntil = snoozeUntil;
          item.updatedAt = updatedAt;
          item.reminderCount = (item.reminderCount || 0) + 1;
        }
        state.lastUpdate = new Date();
      })
      .addCase(snoozeItem.rejected, (state, action) => {
        state.error = action.payload as string;
      });

    // Create chat session
    builder
      .addCase(createChatSession.fulfilled, (state, action) => {
        const { threadId, itemId, messages } = action.payload;
        state.activeSessions[threadId] = {
          threadId,
          itemId,
          messages,
          isTyping: false,
          isConnected: true,
        };
        state.currentChatSession = threadId;

        // Update item with thread information
        const item = state.items.find(item => item.id === itemId);
        if (item) {
          item.threadId = threadId;
        }
      })
      .addCase(createChatSession.rejected, (state, action) => {
        state.error = action.payload as string;
      });

    // Execute task
    builder
      .addCase(executeTask.fulfilled, (state, action) => {
        const { executionId, sessionId, itemId, status } = action.payload;
        state.activeExecutions[executionId] = {
          executionId,
          sessionId,
          itemId,
          status: status === 'started' ? 'running' : 'idle',
          progress: [],
        };

        // Update item execution status
        const item = state.items.find(item => item.id === itemId);
        if (item) {
          item.executionStatus = 'running';
          item.currentSessionId = sessionId;
        }
      })
      .addCase(executeTask.rejected, (state, action) => {
        state.error = action.payload as string;
      });
  },
});

export const {
  setInitialized,
  setConnectionStatus,
  setItems,
  addItem,
  removeItem,
  setSourceFilter,
  setPriorityFilter,
  setSearchFilter,
  setChatSession,
  addMessage,
  setTyping,
  closeChatSession,
  setCurrentChatSession,
  setExecution,
  updateExecutionProgress,
  setExecutionResult,
  clearExecution,
  clearError,
} = intelligenceSlice.actions;

export default intelligenceSlice.reducer;
