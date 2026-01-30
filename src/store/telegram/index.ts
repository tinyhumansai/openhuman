import { createSlice } from "@reduxjs/toolkit";
import type { TelegramRootState } from "./types";
import { reducers } from "./reducers";
import { buildExtraReducers } from "./extraReducers";

const telegramInitialState: TelegramRootState = { byUser: {} };

const telegramSlice = createSlice({
  name: "telegram",
  initialState: telegramInitialState,
  reducers: {
    ...reducers,
  },
  extraReducers: buildExtraReducers,
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
  resetTelegramForUser,
  resetChats,
  resetMessages,
  // Normalized message indexing
  addChatMessagesById,
  setViewportIds,
  addOutlyingList,
  mergeOutlyingLists,
  deleteChatMessages,
  setThreadListedIds,
  // Update sequencing
  setCommonBoxState,
  setChannelPts,
} = telegramSlice.actions;

// Re-export thunks
export {
  initializeTelegram,
  connectTelegram,
  checkAuthStatus,
  fetchChats,
  fetchMessages,
} from "./thunks";

// Re-export types
export type {
  TelegramConnectionStatus,
  TelegramAuthStatus,
  TelegramUser,
  TelegramChat,
  TelegramMessage,
  TelegramThread,
  TelegramState,
  TelegramRootState,
  ThreadMessageState,
} from "./types";
export { initialState, MAIN_THREAD_ID } from "./types";

export default telegramSlice.reducer;
