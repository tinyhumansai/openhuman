import { createSlice, PayloadAction, createAsyncThunk } from '@reduxjs/toolkit';
import { mtprotoService } from '../services/mtprotoService';
import type { Api } from 'telegram/tl';

// Types for Telegram entities
export type TelegramConnectionStatus = 'disconnected' | 'connecting' | 'connected' | 'error';
export type TelegramAuthStatus = 'not_authenticated' | 'authenticating' | 'authenticated' | 'error';

export interface TelegramUser {
  id: string;
  firstName: string;
  lastName?: string;
  username?: string;
  phoneNumber?: string;
  isBot: boolean;
  isVerified?: boolean;
  isPremium?: boolean;
  accessHash?: string;
}

export interface TelegramChat {
  id: string;
  title?: string;
  type: 'private' | 'group' | 'supergroup' | 'channel';
  username?: string;
  accessHash?: string;
  unreadCount: number;
  lastMessage?: TelegramMessage;
  lastMessageDate?: number;
  isPinned: boolean;
  photo?: {
    smallFileId?: string;
    bigFileId?: string;
  };
  participantsCount?: number;
}

export interface TelegramMessage {
  id: string;
  chatId: string;
  threadId?: string;
  date: number;
  message: string;
  fromId?: string;
  fromName?: string;
  isOutgoing: boolean;
  isEdited: boolean;
  isForwarded: boolean;
  replyToMessageId?: string;
  media?: {
    type: string;
    [key: string]: unknown;
  };
  reactions?: Array<{
    emoticon: string;
    count: number;
  }>;
  views?: number;
}

export interface TelegramThread {
  id: string;
  chatId: string;
  title: string;
  messageCount: number;
  lastMessage?: TelegramMessage;
  lastMessageDate?: number;
  unreadCount: number;
  isPinned: boolean;
}

interface TelegramState {
  // Connection state
  connectionStatus: TelegramConnectionStatus;
  connectionError: string | null;

  // Authentication state
  authStatus: TelegramAuthStatus;
  authError: string | null;
  isInitialized: boolean;
  phoneNumber: string | null;
  sessionString: string | null;

  // User data
  currentUser: TelegramUser | null;

  // Chats
  chats: Record<string, TelegramChat>;
  chatsOrder: string[]; // Ordered list of chat IDs
  selectedChatId: string | null;

  // Messages (organized by chatId)
  messages: Record<string, Record<string, TelegramMessage>>; // [chatId][messageId] = message
  messagesOrder: Record<string, string[]>; // [chatId] = [messageId, ...]

  // Threads (organized by chatId)
  threads: Record<string, Record<string, TelegramThread>>; // [chatId][threadId] = thread
  threadsOrder: Record<string, string[]>; // [chatId] = [threadId, ...]
  selectedThreadId: string | null;

  // Loading states
  isLoadingChats: boolean;
  isLoadingMessages: boolean;
  isLoadingThreads: boolean;

  // Pagination
  hasMoreChats: boolean;
  hasMoreMessages: Record<string, boolean>; // [chatId] = hasMore
  hasMoreThreads: Record<string, boolean>; // [chatId] = hasMore

  // Filters and search
  searchQuery: string | null;
  filteredChatIds: string[] | null;
}

const initialState: TelegramState = {
  // Connection
  connectionStatus: 'disconnected',
  connectionError: null,

  // Authentication
  authStatus: 'not_authenticated',
  authError: null,
  isInitialized: false,
  phoneNumber: null,
  sessionString: null,

  // User
  currentUser: null,

  // Chats
  chats: {},
  chatsOrder: [],
  selectedChatId: null,

  // Messages
  messages: {},
  messagesOrder: {},

  // Threads
  threads: {},
  threadsOrder: {},
  selectedThreadId: null,

  // Loading
  isLoadingChats: false,
  isLoadingMessages: false,
  isLoadingThreads: false,

  // Pagination
  hasMoreChats: true,
  hasMoreMessages: {},
  hasMoreThreads: {},

  // Search
  searchQuery: null,
  filteredChatIds: null,
};

// Async thunks
export const initializeTelegram = createAsyncThunk(
  'telegram/initialize',
  async (_, { rejectWithValue }) => {
    try {
      await mtprotoService.initialize();
      const sessionString = mtprotoService.getSessionString();
      return { sessionString };
    } catch (error) {
      return rejectWithValue(
        error instanceof Error ? error.message : 'Failed to initialize Telegram client'
      );
    }
  }
);

export const connectTelegram = createAsyncThunk(
  'telegram/connect',
  async (_, { rejectWithValue }) => {
    try {
      await mtprotoService.connect();
      return true;
    } catch (error) {
      return rejectWithValue(
        error instanceof Error ? error.message : 'Failed to connect to Telegram'
      );
    }
  }
);

export const checkAuthStatus = createAsyncThunk(
  'telegram/checkAuthStatus',
  async (_, { rejectWithValue }) => {
    try {
      const client = mtprotoService.getClient();
      const isAuthorized = await client.checkAuthorization();
      
      if (isAuthorized) {
        const me = await client.getMe();
        return me;
      }
      return null;
    } catch (error) {
      return rejectWithValue(
        error instanceof Error ? error.message : 'Failed to check auth status'
      );
    }
  }
);

export const fetchChats = createAsyncThunk(
  'telegram/fetchChats',
  async (_, { rejectWithValue }) => {
    try {
      const client = mtprotoService.getClient();
      const dialogs = await client.getDialogs({ limit: 100 });
      return dialogs;
    } catch (error) {
      return rejectWithValue(
        error instanceof Error ? error.message : 'Failed to fetch chats'
      );
    }
  }
);

export const fetchMessages = createAsyncThunk(
  'telegram/fetchMessages',
  async (
    { chatId, limit = 50, offsetId }: { chatId: string; limit?: number; offsetId?: number },
    { rejectWithValue }
  ) => {
    try {
      const client = mtprotoService.getClient();
      // Implementation depends on GramJS API
      // This is a placeholder - adjust based on actual API
      const messages = await client.getMessages(chatId, { limit, offsetId });
      return { chatId, messages };
    } catch (error) {
      return rejectWithValue(
        error instanceof Error ? error.message : 'Failed to fetch messages'
      );
    }
  }
);

const telegramSlice = createSlice({
  name: 'telegram',
  initialState,
  reducers: {
    // Connection actions
    setConnectionStatus: (state, action: PayloadAction<TelegramConnectionStatus>) => {
      state.connectionStatus = action.payload;
      if (action.payload !== 'error') {
        state.connectionError = null;
      }
    },
    setConnectionError: (state, action: PayloadAction<string | null>) => {
      state.connectionError = action.payload;
      if (action.payload) {
        state.connectionStatus = 'error';
      }
    },

    // Authentication actions
    setAuthStatus: (state, action: PayloadAction<TelegramAuthStatus>) => {
      state.authStatus = action.payload;
      if (action.payload !== 'error') {
        state.authError = null;
      }
    },
    setAuthError: (state, action: PayloadAction<string | null>) => {
      state.authError = action.payload;
      if (action.payload) {
        state.authStatus = 'error';
      }
    },
    setPhoneNumber: (state, action: PayloadAction<string | null>) => {
      state.phoneNumber = action.payload;
    },
    setSessionString: (state, action: PayloadAction<string | null>) => {
      state.sessionString = action.payload;
    },

    // User actions
    setCurrentUser: (state, action: PayloadAction<TelegramUser | null>) => {
      state.currentUser = action.payload;
    },

    // Chat actions
    setChats: (state, action: PayloadAction<Record<string, TelegramChat>>) => {
      state.chats = action.payload;
    },
    addChat: (state, action: PayloadAction<TelegramChat>) => {
      const chat = action.payload;
      state.chats[chat.id] = chat;
      if (!state.chatsOrder.includes(chat.id)) {
        state.chatsOrder.unshift(chat.id);
      }
    },
    updateChat: (state, action: PayloadAction<Partial<TelegramChat> & { id: string }>) => {
      const { id, ...updates } = action.payload;
      if (state.chats[id]) {
        state.chats[id] = { ...state.chats[id], ...updates };
      }
    },
    removeChat: (state, action: PayloadAction<string>) => {
      const chatId = action.payload;
      delete state.chats[chatId];
      state.chatsOrder = state.chatsOrder.filter((id) => id !== chatId);
      if (state.selectedChatId === chatId) {
        state.selectedChatId = null;
      }
    },
    setSelectedChat: (state, action: PayloadAction<string | null>) => {
      state.selectedChatId = action.payload;
      // Clear selected thread when changing chat
      if (action.payload !== state.selectedChatId) {
        state.selectedThreadId = null;
      }
    },
    setChatsOrder: (state, action: PayloadAction<string[]>) => {
      state.chatsOrder = action.payload;
    },

    // Message actions
    addMessage: (state, action: PayloadAction<TelegramMessage>) => {
      const message = action.payload;
      const { chatId, id } = message;

      if (!state.messages[chatId]) {
        state.messages[chatId] = {};
        state.messagesOrder[chatId] = [];
      }

      if (!state.messages[chatId][id]) {
        state.messages[chatId][id] = message;
        state.messagesOrder[chatId].push(id);
      }
    },
    addMessages: (state, action: PayloadAction<{ chatId: string; messages: TelegramMessage[] }>) => {
      const { chatId, messages } = action.payload;

      if (!state.messages[chatId]) {
        state.messages[chatId] = {};
        state.messagesOrder[chatId] = [];
      }

      messages.forEach((message) => {
        if (!state.messages[chatId][message.id]) {
          state.messages[chatId][message.id] = message;
          state.messagesOrder[chatId].push(message.id);
        }
      });
    },
    updateMessage: (
      state,
      action: PayloadAction<{ chatId: string; messageId: string; updates: Partial<TelegramMessage> }>
    ) => {
      const { chatId, messageId, updates } = action.payload;
      if (state.messages[chatId]?.[messageId]) {
        state.messages[chatId][messageId] = {
          ...state.messages[chatId][messageId],
          ...updates,
        };
      }
    },
    removeMessage: (state, action: PayloadAction<{ chatId: string; messageId: string }>) => {
      const { chatId, messageId } = action.payload;
      if (state.messages[chatId]?.[messageId]) {
        delete state.messages[chatId][messageId];
        state.messagesOrder[chatId] = state.messagesOrder[chatId].filter(
          (id) => id !== messageId
        );
      }
    },
    clearMessages: (state, action: PayloadAction<string>) => {
      const chatId = action.payload;
      delete state.messages[chatId];
      delete state.messagesOrder[chatId];
    },

    // Thread actions
    addThread: (state, action: PayloadAction<TelegramThread>) => {
      const thread = action.payload;
      const { chatId, id } = thread;

      if (!state.threads[chatId]) {
        state.threads[chatId] = {};
        state.threadsOrder[chatId] = [];
      }

      if (!state.threads[chatId][id]) {
        state.threads[chatId][id] = thread;
        state.threadsOrder[chatId].push(id);
      }
    },
    updateThread: (
      state,
      action: PayloadAction<{ chatId: string; threadId: string; updates: Partial<TelegramThread> }>
    ) => {
      const { chatId, threadId, updates } = action.payload;
      if (state.threads[chatId]?.[threadId]) {
        state.threads[chatId][threadId] = {
          ...state.threads[chatId][threadId],
          ...updates,
        };
      }
    },
    setSelectedThread: (state, action: PayloadAction<string | null>) => {
      state.selectedThreadId = action.payload;
    },

    // Search actions
    setSearchQuery: (state, action: PayloadAction<string | null>) => {
      state.searchQuery = action.payload;
    },
    setFilteredChatIds: (state, action: PayloadAction<string[] | null>) => {
      state.filteredChatIds = action.payload;
    },

    // Reset actions
    resetTelegram: (state) => {
      return initialState;
    },
    resetChats: (state) => {
      state.chats = {};
      state.chatsOrder = [];
      state.selectedChatId = null;
    },
    resetMessages: (state) => {
      state.messages = {};
      state.messagesOrder = {};
    },
  },
  extraReducers: (builder) => {
    // Initialize
    builder
      .addCase(initializeTelegram.pending, (state) => {
        state.isInitialized = false;
      })
      .addCase(initializeTelegram.fulfilled, (state, action) => {
        state.isInitialized = true;
        state.sessionString = action.payload.sessionString;
      })
      .addCase(initializeTelegram.rejected, (state, action) => {
        state.isInitialized = false;
        state.connectionError = action.payload as string;
      });

    // Connect
    builder
      .addCase(connectTelegram.pending, (state) => {
        state.connectionStatus = 'connecting';
        state.connectionError = null;
      })
      .addCase(connectTelegram.fulfilled, (state) => {
        state.connectionStatus = 'connected';
        state.connectionError = null;
      })
      .addCase(connectTelegram.rejected, (state, action) => {
        state.connectionStatus = 'error';
        state.connectionError = action.payload as string;
      });

    // Check auth
    builder
      .addCase(checkAuthStatus.pending, (state) => {
        state.authStatus = 'authenticating';
      })
      .addCase(checkAuthStatus.fulfilled, (state, action) => {
        if (action.payload) {
          state.authStatus = 'authenticated';
          // Convert Api.User to TelegramUser
          // This is a placeholder - adjust based on actual API response
          state.currentUser = {
            id: String(action.payload.id),
            firstName: action.payload.firstName || '',
            lastName: action.payload.lastName,
            username: action.payload.username,
            isBot: Boolean(action.payload.bot),
            accessHash: action.payload.accessHash?.toString(),
          };
        } else {
          state.authStatus = 'not_authenticated';
          state.currentUser = null;
        }
      })
      .addCase(checkAuthStatus.rejected, (state, action) => {
        state.authStatus = 'error';
        state.authError = action.payload as string;
      });

    // Fetch chats
    builder
      .addCase(fetchChats.pending, (state) => {
        state.isLoadingChats = true;
      })
      .addCase(fetchChats.fulfilled, (state, action) => {
        state.isLoadingChats = false;
        // Convert dialogs to chats
        // This is a placeholder - adjust based on actual API response
        // action.payload should be an array of dialogs
      })
      .addCase(fetchChats.rejected, (state) => {
        state.isLoadingChats = false;
      });

    // Fetch messages
    builder
      .addCase(fetchMessages.pending, (state) => {
        state.isLoadingMessages = true;
      })
      .addCase(fetchMessages.fulfilled, (state, action) => {
        state.isLoadingMessages = false;
        // Messages will be added via addMessages action
      })
      .addCase(fetchMessages.rejected, (state) => {
        state.isLoadingMessages = false;
      });
  },
});

export const {
  setConnectionStatus,
  setConnectionError,
  setAuthStatus,
  setAuthError,
  setPhoneNumber,
  setSessionString,
  setCurrentUser,
  setChats,
  addChat,
  updateChat,
  removeChat,
  setSelectedChat,
  setChatsOrder,
  addMessage,
  addMessages,
  updateMessage,
  removeMessage,
  clearMessages,
  addThread,
  updateThread,
  setSelectedThread,
  setSearchQuery,
  setFilteredChatIds,
  resetTelegram,
  resetChats,
  resetMessages,
} = telegramSlice.actions;

export default telegramSlice.reducer;
