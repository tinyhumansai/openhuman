import { createSelector } from "@reduxjs/toolkit";
import type { RootState } from "./index";
import type {
  TelegramChat,
  TelegramMessage,
  TelegramThread,
  TelegramState,
  ThreadMessageState,
} from "./telegram";
import { initialState, MAIN_THREAD_ID } from "./telegram/types";

function selectCurrentUserId(state: RootState): string {
  return state.user.user?._id ?? "";
}

const defaultUserState: TelegramState = { ...initialState };

export const selectTelegramState = (state: RootState) => state.telegram;

export const selectTelegramUserState = (state: RootState): TelegramState =>
  state.telegram.byUser[selectCurrentUserId(state)] ?? defaultUserState;

export const selectConnectionStatus = (state: RootState) =>
  selectTelegramUserState(state).connectionStatus;
export const selectConnectionError = (state: RootState) =>
  selectTelegramUserState(state).connectionError;
export const selectIsConnected = (state: RootState) =>
  selectTelegramUserState(state).connectionStatus === "connected";

export const selectAuthStatus = (state: RootState) =>
  selectTelegramUserState(state).authStatus;
export const selectAuthError = (state: RootState) =>
  selectTelegramUserState(state).authError;
export const selectIsAuthenticated = (state: RootState) =>
  selectTelegramUserState(state).authStatus === "authenticated";
export const selectIsInitialized = (state: RootState) =>
  selectTelegramUserState(state).isInitialized;
export const selectPhoneNumber = (state: RootState) =>
  selectTelegramUserState(state).phoneNumber;
export const selectCurrentUser = (state: RootState) =>
  selectTelegramUserState(state).currentUser;

export const selectSessionString = (state: RootState) =>
  selectTelegramUserState(state).sessionString;

// Chat selectors
export const selectAllChats = (state: RootState) =>
  selectTelegramUserState(state).chats;
export const selectChatsOrder = (state: RootState) =>
  selectTelegramUserState(state).chatsOrder;
export const selectSelectedChatId = (state: RootState) =>
  selectTelegramUserState(state).selectedChatId;
export const selectIsLoadingChats = (state: RootState) =>
  selectTelegramUserState(state).isLoadingChats;

export const selectOrderedChats = createSelector(
  [selectAllChats, selectChatsOrder],
  (chats, order): TelegramChat[] =>
    order.map((id) => chats[id]).filter(Boolean),
);

export const selectSelectedChat = createSelector(
  [selectAllChats, selectSelectedChatId],
  (chats, selectedId): TelegramChat | null =>
    selectedId ? chats[selectedId] || null : null,
);

export const selectFilteredChats = createSelector(
  [
    selectOrderedChats,
    (state: RootState) => selectTelegramUserState(state).filteredChatIds,
  ],
  (chats, filteredIds): TelegramChat[] => {
    if (!filteredIds) return chats;
    return chats.filter((chat) => filteredIds.includes(chat.id));
  },
);

// Message selectors
export const selectAllMessages = (state: RootState) =>
  selectTelegramUserState(state).messages;
export const selectMessagesOrder = (state: RootState) =>
  selectTelegramUserState(state).messagesOrder;
export const selectIsLoadingMessages = (state: RootState) =>
  selectTelegramUserState(state).isLoadingMessages;

export const selectChatMessages = createSelector(
  [
    selectAllMessages,
    selectMessagesOrder,
    (_: RootState, chatId: string) => chatId,
  ],
  (messages, messagesOrder, chatId): TelegramMessage[] => {
    const chatMessages = messages[chatId];
    const order = messagesOrder[chatId] || [];
    if (!chatMessages) return [];
    return order.map((id) => chatMessages[id]).filter(Boolean);
  },
);

export const selectChatLatestMessage = createSelector(
  [selectChatMessages],
  (messages): TelegramMessage | null =>
    messages.length > 0 ? messages[messages.length - 1] : null,
);

// Thread selectors
export const selectAllThreads = (state: RootState) =>
  selectTelegramUserState(state).threads;
export const selectThreadsOrder = (state: RootState) =>
  selectTelegramUserState(state).threadsOrder;
export const selectSelectedThreadId = (state: RootState) =>
  selectTelegramUserState(state).selectedThreadId;
export const selectIsLoadingThreads = (state: RootState) =>
  selectTelegramUserState(state).isLoadingThreads;

export const selectChatThreads = createSelector(
  [
    selectAllThreads,
    selectThreadsOrder,
    (_: RootState, chatId: string) => chatId,
  ],
  (threads, threadsOrder, chatId): TelegramThread[] => {
    const chatThreads = threads[chatId];
    const order = threadsOrder[chatId] || [];
    if (!chatThreads) return [];
    return order.map((id) => chatThreads[id]).filter(Boolean);
  },
);

export const selectSelectedThread = createSelector(
  [selectAllThreads, selectSelectedChatId, selectSelectedThreadId],
  (threads, chatId, threadId): TelegramThread | null => {
    if (!chatId || !threadId) return null;
    return threads[chatId]?.[threadId] || null;
  },
);

export const selectThreadMessages = createSelector(
  [selectChatMessages, selectSelectedChatId, selectSelectedThreadId],
  (messages, _chatId, threadId): TelegramMessage[] => {
    if (!threadId) return [];
    return messages.filter((msg) => msg.threadId === threadId);
  },
);

// Search selectors
export const selectSearchQuery = (state: RootState) =>
  selectTelegramUserState(state).searchQuery;
export const selectIsSearching = (state: RootState) =>
  selectTelegramUserState(state).searchQuery !== null;

// Pagination selectors
export const selectHasMoreChats = (state: RootState) =>
  selectTelegramUserState(state).hasMoreChats;
export const selectHasMoreMessages = (state: RootState) =>
  selectTelegramUserState(state).hasMoreMessages;
export const selectHasMoreThreads = (state: RootState) =>
  selectTelegramUserState(state).hasMoreThreads;

export const selectChatHasMoreMessages = createSelector(
  [selectHasMoreMessages, (_: RootState, chatId: string) => chatId],
  (hasMore, chatId) => hasMore[chatId] ?? true,
);

export const selectChatHasMoreThreads = createSelector(
  [selectHasMoreThreads, (_: RootState, chatId: string) => chatId],
  (hasMore, chatId) => hasMore[chatId] ?? true,
);

export const selectTelegramReady = createSelector(
  [selectIsConnected, selectIsAuthenticated, selectIsInitialized],
  (isConnected, isAuthenticated, isInitialized) =>
    isConnected && isAuthenticated && isInitialized,
);

export const selectTotalUnreadCount = createSelector(
  [selectOrderedChats],
  (chats) => chats.reduce((total, chat) => total + (chat.unreadCount || 0), 0),
);

export const selectPinnedChats = createSelector(
  [selectOrderedChats],
  (chats): TelegramChat[] => chats.filter((chat) => chat.isPinned),
);

// ---------------------------------------------------------------------------
// Normalized message indexing selectors
// ---------------------------------------------------------------------------

const emptyThreadState: ThreadMessageState = {
  listedIds: [],
  outlyingLists: [],
  viewportIds: [],
};

/** Select the thread index map for a given chat */
export const selectChatThreadIndex = (state: RootState, chatId: string) =>
  selectTelegramUserState(state).threadIndex[chatId];

/** Select the thread message state for a given chat + thread */
export const selectThreadMessageState = (
  state: RootState,
  chatId: string,
  threadId: string = MAIN_THREAD_ID,
): ThreadMessageState =>
  selectTelegramUserState(state).threadIndex[chatId]?.[threadId] ??
  emptyThreadState;

/** O(1) lookup: get a single message by chat ID and message ID */
export const selectChatMessageById = (
  state: RootState,
  chatId: string,
  messageId: string,
): TelegramMessage | undefined =>
  selectTelegramUserState(state).messages[chatId]?.[messageId];

/** Select the listed (loaded) IDs for a chat's main thread */
export const selectChatListedIds = createSelector(
  [
    (state: RootState) => selectTelegramUserState(state).messagesOrder,
    (_: RootState, chatId: string) => chatId,
  ],
  (messagesOrder, chatId): string[] => messagesOrder[chatId] ?? [],
);

/** Select viewport IDs for a chat + thread */
export const selectChatViewportIds = createSelector(
  [
    (state: RootState) => selectTelegramUserState(state).threadIndex,
    (_: RootState, chatId: string) => chatId,
    (_: RootState, _chatId: string, threadId?: string) =>
      threadId ?? MAIN_THREAD_ID,
  ],
  (threadIndex, chatId, threadId): string[] =>
    threadIndex[chatId]?.[threadId]?.viewportIds ?? [],
);

/** Select outlying (disjoint) lists for a chat + thread */
export const selectChatOutlyingLists = createSelector(
  [
    (state: RootState) => selectTelegramUserState(state).threadIndex,
    (_: RootState, chatId: string) => chatId,
    (_: RootState, _chatId: string, threadId?: string) =>
      threadId ?? MAIN_THREAD_ID,
  ],
  (threadIndex, chatId, threadId): string[][] =>
    threadIndex[chatId]?.[threadId]?.outlyingLists ?? [],
);

/** Resolve viewport IDs to full message objects */
export const selectViewportMessages = createSelector(
  [
    (state: RootState) => selectTelegramUserState(state).messages,
    (state: RootState) => selectTelegramUserState(state).threadIndex,
    (_: RootState, chatId: string) => chatId,
    (_: RootState, _chatId: string, threadId?: string) =>
      threadId ?? MAIN_THREAD_ID,
  ],
  (messages, threadIndex, chatId, threadId): TelegramMessage[] => {
    const ids = threadIndex[chatId]?.[threadId]?.viewportIds ?? [];
    const byId = messages[chatId] ?? {};
    return ids.map((id) => byId[id]).filter(Boolean);
  },
);

export { selectCurrentUserId as selectTelegramCurrentUserId };
