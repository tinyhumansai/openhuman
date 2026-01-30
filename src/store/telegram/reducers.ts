import { PayloadAction } from "@reduxjs/toolkit";
import type {
  TelegramRootState,
  TelegramState,
  TelegramConnectionStatus,
  TelegramAuthStatus,
  TelegramUser,
  TelegramChat,
  TelegramMessage,
  TelegramThread,
  ThreadMessageState,
} from "./types";
import { initialState, MAIN_THREAD_ID } from "./types";

function ensureUser(
  state: TelegramRootState,
  userId: string,
): TelegramState {
  if (!state.byUser[userId]) {
    state.byUser[userId] = { ...initialState };
  }
  return state.byUser[userId];
}

function ensureThreadIndex(
  u: TelegramState,
  chatId: string,
  threadId: string,
): ThreadMessageState {
  if (!u.threadIndex[chatId]) {
    u.threadIndex[chatId] = {};
  }
  if (!u.threadIndex[chatId][threadId]) {
    u.threadIndex[chatId][threadId] = {
      listedIds: [],
      outlyingLists: [],
      viewportIds: [],
    };
  }
  return u.threadIndex[chatId][threadId];
}

export const reducers = {
  setConnectionStatus: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string; status: TelegramConnectionStatus }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    u.connectionStatus = action.payload.status;
    if (action.payload.status !== "error") u.connectionError = null;
  },
  setConnectionError: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string; error: string | null }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    u.connectionError = action.payload.error;
    if (action.payload.error) u.connectionStatus = "error";
  },
  setAuthStatus: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string; status: TelegramAuthStatus }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    u.authStatus = action.payload.status;
    if (action.payload.status !== "error") u.authError = null;
  },
  setAuthError: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string; error: string | null }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    u.authError = action.payload.error;
    if (action.payload.error) u.authStatus = "error";
  },
  setPhoneNumber: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string; phoneNumber: string | null }>,
  ) => {
    ensureUser(state, action.payload.userId).phoneNumber =
      action.payload.phoneNumber;
  },
  setSessionString: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string; sessionString: string | null }>,
  ) => {
    ensureUser(state, action.payload.userId).sessionString =
      action.payload.sessionString;
  },
  setCurrentUser: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string; user: TelegramUser | null }>,
  ) => {
    ensureUser(state, action.payload.userId).currentUser = action.payload.user;
  },
  setChats: (
    state: TelegramRootState,
    action: PayloadAction<{
      userId: string;
      chats: Record<string, TelegramChat>;
    }>,
  ) => {
    ensureUser(state, action.payload.userId).chats = action.payload.chats;
  },
  addChat: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string; chat: TelegramChat }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    const chat = action.payload.chat;
    u.chats[chat.id] = chat;
    if (!u.chatsOrder.includes(chat.id)) u.chatsOrder.unshift(chat.id);
  },
  updateChat: (
    state: TelegramRootState,
    action: PayloadAction<{
      userId: string;
      id: string;
      updates: Partial<TelegramChat>;
    }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    const { id, updates } = action.payload;
    if (u.chats[id]) u.chats[id] = { ...u.chats[id], ...updates };
  },
  removeChat: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string; chatId: string }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    const chatId = action.payload.chatId;
    delete u.chats[chatId];
    u.chatsOrder = u.chatsOrder.filter((id) => id !== chatId);
    if (u.selectedChatId === chatId) u.selectedChatId = null;
  },
  setSelectedChat: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string; chatId: string | null }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    const prev = u.selectedChatId;
    u.selectedChatId = action.payload.chatId;
    if (action.payload.chatId !== prev) u.selectedThreadId = null;
  },
  setChatsOrder: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string; order: string[] }>,
  ) => {
    ensureUser(state, action.payload.userId).chatsOrder = action.payload.order;
  },
  addMessage: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string; message: TelegramMessage }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    const { chatId, id } = action.payload.message;
    if (!u.messages[chatId]) {
      u.messages[chatId] = {};
      u.messagesOrder[chatId] = [];
    }
    if (!u.messages[chatId][id]) {
      u.messages[chatId][id] = action.payload.message;
      u.messagesOrder[chatId].push(id);
    }
  },
  addMessages: (
    state: TelegramRootState,
    action: PayloadAction<{
      userId: string;
      chatId: string;
      messages: TelegramMessage[];
    }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    const { chatId, messages } = action.payload;
    if (!u.messages[chatId]) {
      u.messages[chatId] = {};
      u.messagesOrder[chatId] = [];
    }
    messages.forEach((m) => {
      if (!u.messages[chatId][m.id]) {
        u.messages[chatId][m.id] = m;
        u.messagesOrder[chatId].push(m.id);
      }
    });
  },
  updateMessage: (
    state: TelegramRootState,
    action: PayloadAction<{
      userId: string;
      chatId: string;
      messageId: string;
      updates: Partial<TelegramMessage>;
    }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    const { chatId, messageId, updates } = action.payload;
    if (u.messages[chatId]?.[messageId]) {
      u.messages[chatId][messageId] = {
        ...u.messages[chatId][messageId],
        ...updates,
      };
    }
  },
  removeMessage: (
    state: TelegramRootState,
    action: PayloadAction<{
      userId: string;
      chatId: string;
      messageId: string;
    }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    const { chatId, messageId } = action.payload;
    if (u.messages[chatId]?.[messageId]) {
      delete u.messages[chatId][messageId];
      u.messagesOrder[chatId] = u.messagesOrder[chatId].filter(
        (id) => id !== messageId,
      );
    }
  },
  clearMessages: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string; chatId: string }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    delete u.messages[action.payload.chatId];
    delete u.messagesOrder[action.payload.chatId];
  },
  addThread: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string; thread: TelegramThread }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    const { chatId, id } = action.payload.thread;
    if (!u.threads[chatId]) {
      u.threads[chatId] = {};
      u.threadsOrder[chatId] = [];
    }
    if (!u.threads[chatId][id]) {
      u.threads[chatId][id] = action.payload.thread;
      u.threadsOrder[chatId].push(id);
    }
  },
  updateThread: (
    state: TelegramRootState,
    action: PayloadAction<{
      userId: string;
      chatId: string;
      threadId: string;
      updates: Partial<TelegramThread>;
    }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    const { chatId, threadId, updates } = action.payload;
    if (u.threads[chatId]?.[threadId]) {
      u.threads[chatId][threadId] = {
        ...u.threads[chatId][threadId],
        ...updates,
      };
    }
  },
  setSelectedThread: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string; threadId: string | null }>,
  ) => {
    ensureUser(state, action.payload.userId).selectedThreadId =
      action.payload.threadId;
  },
  setSearchQuery: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string; query: string | null }>,
  ) => {
    ensureUser(state, action.payload.userId).searchQuery = action.payload.query;
  },
  setFilteredChatIds: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string; chatIds: string[] | null }>,
  ) => {
    ensureUser(state, action.payload.userId).filteredChatIds =
      action.payload.chatIds;
  },
  resetChats: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    u.chats = {};
    u.chatsOrder = [];
    u.selectedChatId = null;
  },
  resetMessages: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    u.messages = {};
    u.messagesOrder = {};
  },
  resetTelegramForUser: (
    state: TelegramRootState,
    action: PayloadAction<{ userId: string }>,
  ) => {
    state.byUser[action.payload.userId] = { ...initialState };
  },

  // ---------------------------------------------------------------------------
  // Normalized message indexing reducers
  // ---------------------------------------------------------------------------

  /**
   * Bulk-add messages into the normalized byId map for a chat.
   * Also appends new IDs to messagesOrder (listedIds equivalent) and
   * the main thread index, de-duplicating and maintaining chronological order.
   */
  addChatMessagesById: (
    state: TelegramRootState,
    action: PayloadAction<{
      userId: string;
      chatId: string;
      messages: TelegramMessage[];
    }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    const { chatId, messages } = action.payload;

    if (!u.messages[chatId]) {
      u.messages[chatId] = {};
      u.messagesOrder[chatId] = [];
    }

    const existingIds = new Set(u.messagesOrder[chatId]);
    const newIds: string[] = [];

    for (const msg of messages) {
      u.messages[chatId][msg.id] = msg;
      if (!existingIds.has(msg.id)) {
        newIds.push(msg.id);
        existingIds.add(msg.id);
      }
    }

    if (newIds.length > 0) {
      // Merge into order maintaining chronological sort (Telegram IDs are sequential)
      u.messagesOrder[chatId].push(...newIds);
      u.messagesOrder[chatId].sort((a, b) => {
        const msgA = u.messages[chatId][a];
        const msgB = u.messages[chatId][b];
        if (!msgA || !msgB) return 0;
        return msgA.date - msgB.date;
      });

      // Also update the main thread index
      ensureThreadIndex(u, chatId, MAIN_THREAD_ID);
      u.threadIndex[chatId][MAIN_THREAD_ID].listedIds =
        u.messagesOrder[chatId];
    }
  },

  /**
   * Set the viewport IDs for a chat + thread.
   * Viewport = currently visible messages, capped by the caller.
   */
  setViewportIds: (
    state: TelegramRootState,
    action: PayloadAction<{
      userId: string;
      chatId: string;
      threadId?: string;
      viewportIds: string[];
    }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    const { chatId, viewportIds } = action.payload;
    const threadId = action.payload.threadId ?? MAIN_THREAD_ID;
    ensureThreadIndex(u, chatId, threadId);
    u.threadIndex[chatId][threadId].viewportIds = viewportIds;
  },

  /**
   * Add an outlying (disjoint) list of message IDs for a chat + thread.
   * Used when the user jumps to a message far from the current viewport.
   */
  addOutlyingList: (
    state: TelegramRootState,
    action: PayloadAction<{
      userId: string;
      chatId: string;
      threadId?: string;
      ids: string[];
    }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    const { chatId, ids } = action.payload;
    const threadId = action.payload.threadId ?? MAIN_THREAD_ID;
    ensureThreadIndex(u, chatId, threadId);
    u.threadIndex[chatId][threadId].outlyingLists.push(ids);
  },

  /**
   * Merge overlapping outlying lists for a chat + thread.
   * Call after fetching messages that bridge a gap between ranges.
   */
  mergeOutlyingLists: (
    state: TelegramRootState,
    action: PayloadAction<{
      userId: string;
      chatId: string;
      threadId?: string;
    }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    const { chatId } = action.payload;
    const threadId = action.payload.threadId ?? MAIN_THREAD_ID;
    const ti = u.threadIndex[chatId]?.[threadId];
    if (!ti || ti.outlyingLists.length < 2) return;

    // Convert each list to a set of IDs, then merge overlapping sets
    const idSets: Set<string>[] = ti.outlyingLists.map((l) => new Set(l));
    const merged: Set<string>[] = [];

    for (const current of idSets) {
      let didMerge = false;
      for (let i = 0; i < merged.length; i++) {
        // If any IDs overlap, merge the sets
        for (const id of current) {
          if (merged[i].has(id)) {
            for (const cId of current) merged[i].add(cId);
            didMerge = true;
            break;
          }
        }
        if (didMerge) break;
      }
      if (!didMerge) merged.push(current);
    }

    ti.outlyingLists = merged.map((s) => [...s].sort());
  },

  /**
   * Bulk delete messages from a chat by ID.
   * Removes from byId, messagesOrder, and all thread indices.
   */
  deleteChatMessages: (
    state: TelegramRootState,
    action: PayloadAction<{
      userId: string;
      chatId: string;
      messageIds: string[];
    }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    const { chatId, messageIds } = action.payload;
    if (!u.messages[chatId]) return;

    const toDelete = new Set(messageIds);
    for (const id of messageIds) {
      delete u.messages[chatId][id];
    }
    u.messagesOrder[chatId] = (u.messagesOrder[chatId] ?? []).filter(
      (id) => !toDelete.has(id),
    );

    // Clean up thread indices
    if (u.threadIndex[chatId]) {
      for (const threadId of Object.keys(u.threadIndex[chatId])) {
        const ti = u.threadIndex[chatId][threadId];
        ti.listedIds = ti.listedIds.filter((id) => !toDelete.has(id));
        ti.viewportIds = ti.viewportIds.filter((id) => !toDelete.has(id));
        ti.outlyingLists = ti.outlyingLists
          .map((list) => list.filter((id) => !toDelete.has(id)))
          .filter((list) => list.length > 0);
      }
    }
  },

  /**
   * Update the listed IDs for a specific thread within a chat.
   * Used when thread-specific message fetches complete.
   */
  setThreadListedIds: (
    state: TelegramRootState,
    action: PayloadAction<{
      userId: string;
      chatId: string;
      threadId: string;
      listedIds: string[];
    }>,
  ) => {
    const u = ensureUser(state, action.payload.userId);
    const { chatId, threadId, listedIds } = action.payload;
    ensureThreadIndex(u, chatId, threadId);
    u.threadIndex[chatId][threadId].listedIds = listedIds;
  },

  // ---------------------------------------------------------------------------
  // Update sequencing (pts/qts/seq) reducers
  // ---------------------------------------------------------------------------

  /**
   * Set the common box state (seq, pts, qts, date).
   */
  setCommonBoxState: (
    state: TelegramRootState,
    action: PayloadAction<{
      userId: string;
      commonBoxState: { seq: number; date: number; pts: number; qts: number };
    }>,
  ) => {
    ensureUser(state, action.payload.userId).commonBoxState =
      action.payload.commonBoxState;
  },

  /**
   * Set the PTS for a specific channel.
   */
  setChannelPts: (
    state: TelegramRootState,
    action: PayloadAction<{
      userId: string;
      channelId: string;
      pts: number;
    }>,
  ) => {
    ensureUser(state, action.payload.userId).channelPtsById[
      action.payload.channelId
    ] = action.payload.pts;
  },
};
