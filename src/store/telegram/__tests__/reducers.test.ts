import { configureStore } from "@reduxjs/toolkit";
import { describe, it, expect, beforeEach, vi } from "vitest";

// Mock redux-persist to avoid persistence layer
vi.mock("redux-persist", async () => {
  const actual = await vi.importActual("redux-persist");
  return {
    ...actual as object,
    persistReducer: vi.fn((_config, reducer) => reducer),
    persistStore: vi.fn(() => ({
      purge: vi.fn(),
      flush: vi.fn(),
      pause: vi.fn(),
      persist: vi.fn(),
      rehydrate: vi.fn(),
    })),
  };
});

// Mock the main store exports to prevent store initialization
vi.mock("../../index", () => ({
  store: { getState: vi.fn(), dispatch: vi.fn(), subscribe: vi.fn(), replaceReducer: vi.fn() },
  persistor: { purge: vi.fn(), flush: vi.fn() },
}));

// Now safe to import the telegram slice
import telegramReducer, {
  addChat,
  updateChat,
  removeChat,
  replaceChats,
  addChats,
  setChatsOrder,
  addMessage,
  addMessages,
  updateMessage,
  removeMessage,
  clearMessages,
  deleteChatMessages,
  addChatMessagesById,
  addThread,
  updateThread,
  setSelectedThread,
  setConnectionStatus,
  setConnectionError,
  setAuthStatus,
  setAuthError,
  setSyncStatus,
  setCommonBoxState,
  setChannelPts,
  resetTelegramForUser,
  resetChats,
  resetMessages,
  setViewportIds,
  addOutlyingList,
  setThreadListedIds,
  setSelectedChat,
  setPhoneNumber,
  setSessionString,
  setCurrentUser,
  setUsers,
  addUsers,
} from "../index";

function createStore() {
  return configureStore({ reducer: { telegram: telegramReducer } });
}

const userId = "u1";

describe("Telegram Reducers", () => {
  describe("Chat Management", () => {
    let store: ReturnType<typeof createStore>;

    beforeEach(() => {
      store = createStore();
    });

    it("addChat: adds chat and prepends to chatsOrder", () => {
      const chat = { id: "chat1", type: "private" as const, title: "Chat 1", unreadCount: 0, isPinned: false };

      store.dispatch(addChat({ userId, chat }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.chats.chat1).toEqual(chat);
      expect(state.chatsOrder).toEqual(["chat1"]);
    });

    it("addChat: prepends new chat to existing order", () => {
      store.dispatch(addChat({ userId, chat: { id: "chat1", type: "private" as const, unreadCount: 0, isPinned: false } }));
      store.dispatch(addChat({ userId, chat: { id: "chat2", type: "private" as const, unreadCount: 0, isPinned: false } }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.chatsOrder).toEqual(["chat2", "chat1"]);
    });

    it("addChat: overwrites existing chat", () => {
      const chat = { id: "chat1", type: "private" as const, title: "Original", unreadCount: 0, isPinned: false };
      store.dispatch(addChat({ userId, chat }));
      store.dispatch(addChat({ userId, chat: { id: "chat1", type: "private" as const, title: "Updated", unreadCount: 0, isPinned: false } }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.chats.chat1.title).toBe("Updated");
      expect(state.chatsOrder).toEqual(["chat1"]); // Should not add duplicate to order
    });

    it("updateChat: updates existing chat fields", () => {
      const chat = { id: "chat1", type: "private" as const, title: "Original", unreadCount: 0, isPinned: false };
      store.dispatch(addChat({ userId, chat }));
      store.dispatch(updateChat({ userId, id: "chat1", updates: { title: "Updated" } }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.chats.chat1.title).toBe("Updated");
      expect(state.chats.chat1.type).toBe("private");
    });

    it("updateChat: does nothing if chat does not exist", () => {
      store.dispatch(updateChat({ userId, id: "nonexistent", updates: { title: "Test" } }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.chats.nonexistent).toBeUndefined();
    });

    it("removeChat: removes from chats and chatsOrder", () => {
      store.dispatch(addChat({ userId, chat: { id: "chat1", type: "private" as const, unreadCount: 0, isPinned: false } }));
      store.dispatch(addChat({ userId, chat: { id: "chat2", type: "private" as const, unreadCount: 0, isPinned: false } }));
      store.dispatch(removeChat({ userId, chatId: "chat1" }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.chats.chat1).toBeUndefined();
      expect(state.chatsOrder).toEqual(["chat2"]);
    });

    it("removeChat: clears selectedChatId if it was selected", () => {
      store.dispatch(addChat({ userId, chat: { id: "chat1", type: "private" as const, unreadCount: 0, isPinned: false } }));
      store.dispatch(setSelectedChat({ userId, chatId: "chat1" }));
      store.dispatch(removeChat({ userId, chatId: "chat1" }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.selectedChatId).toBeNull();
    });

    it("removeChat: keeps selectedChatId if different chat removed", () => {
      store.dispatch(addChat({ userId, chat: { id: "chat1", type: "private" as const, unreadCount: 0, isPinned: false } }));
      store.dispatch(addChat({ userId, chat: { id: "chat2", type: "private" as const, unreadCount: 0, isPinned: false } }));
      store.dispatch(setSelectedChat({ userId, chatId: "chat1" }));
      store.dispatch(removeChat({ userId, chatId: "chat2" }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.selectedChatId).toBe("chat1");
    });

    it("replaceChats: replaces all chats and order", () => {
      store.dispatch(addChat({ userId, chat: { id: "chat1", type: "private" as const, unreadCount: 0, isPinned: false } }));

      const newChats = {
        chat2: { id: "chat2", type: "group" as const, unreadCount: 0, isPinned: false },
        chat3: { id: "chat3", type: "channel" as const, unreadCount: 0, isPinned: false },
      };
      const newOrder = ["chat2", "chat3"];

      store.dispatch(replaceChats({ userId, chats: newChats, chatsOrder: newOrder }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.chats).toEqual(newChats);
      expect(state.chatsOrder).toEqual(newOrder);
    });

    it("addChats: appends new chats and updates order", () => {
      store.dispatch(addChat({ userId, chat: { id: "chat1", type: "private" as const, unreadCount: 0, isPinned: false } }));

      const newChats = {
        chat2: { id: "chat2", type: "group" as const, unreadCount: 0, isPinned: false },
        chat3: { id: "chat3", type: "channel" as const, unreadCount: 0, isPinned: false },
      };
      const newOrder = ["chat2", "chat3"];

      store.dispatch(addChats({ userId, chats: newChats, appendOrder: newOrder }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.chats.chat1).toBeDefined();
      expect(state.chats.chat2).toBeDefined();
      expect(state.chats.chat3).toBeDefined();
      expect(state.chatsOrder).toEqual(["chat1", "chat2", "chat3"]);
    });

    it("addChats: skips duplicates in order", () => {
      store.dispatch(addChat({ userId, chat: { id: "chat1", type: "private" as const, unreadCount: 0, isPinned: false } }));

      const newChats = {
        chat1: { id: "chat1", type: "private" as const, unreadCount: 0, isPinned: false },
        chat2: { id: "chat2", type: "group" as const, unreadCount: 0, isPinned: false },
      };
      const newOrder = ["chat1", "chat2"];

      store.dispatch(addChats({ userId, chats: newChats, appendOrder: newOrder }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.chatsOrder).toEqual(["chat1", "chat2"]);
    });

    it("setChatsOrder: replaces order completely", () => {
      store.dispatch(addChat({ userId, chat: { id: "chat1", type: "private" as const, unreadCount: 0, isPinned: false } }));
      store.dispatch(addChat({ userId, chat: { id: "chat2", type: "private" as const, unreadCount: 0, isPinned: false } }));

      store.dispatch(setChatsOrder({ userId, order: ["chat2", "chat1"] }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.chatsOrder).toEqual(["chat2", "chat1"]);
    });
  });

  describe("Message Management", () => {
    let store: ReturnType<typeof createStore>;

    beforeEach(() => {
      store = createStore();
      store.dispatch(addChat({ userId, chat: { id: "chat1", type: "private" as const, unreadCount: 0, isPinned: false } }));
    });

    it("addMessage: adds to messages and messagesOrder", () => {
      const message = { id: "msg1", chatId: "chat1", date: 1000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false };

      store.dispatch(addMessage({ userId, message }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.messages.chat1?.msg1).toEqual(message);
      expect(state.messagesOrder.chat1).toEqual(["msg1"]);
    });

    it("addMessage: skips duplicate message", () => {
      const message = { id: "msg1", chatId: "chat1", date: 1000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false };

      store.dispatch(addMessage({ userId, message }));
      store.dispatch(addMessage({ userId, message }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.messagesOrder.chat1).toEqual(["msg1"]);
    });

    it("addMessage: appends to existing messages", () => {
      store.dispatch(addMessage({ userId, message: { id: "msg1", chatId: "chat1", date: 1000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false } }));
      store.dispatch(addMessage({ userId, message: { id: "msg2", chatId: "chat1", date: 2000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false } }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.messagesOrder.chat1).toEqual(["msg1", "msg2"]);
    });

    it("addMessages: bulk add messages, skips duplicates", () => {
      const messages = [
        { id: "msg1", chatId: "chat1", date: 1000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false },
        { id: "msg2", chatId: "chat1", date: 2000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false },
        { id: "msg3", chatId: "chat1", date: 3000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false },
      ];

      store.dispatch(addMessages({ userId, chatId: "chat1", messages }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.messages.chat1?.msg1).toBeDefined();
      expect(state.messages.chat1?.msg2).toBeDefined();
      expect(state.messages.chat1?.msg3).toBeDefined();
      expect(state.messagesOrder.chat1).toEqual(["msg1", "msg2", "msg3"]);
    });

    it("addMessages: skips duplicates when adding to existing messages", () => {
      store.dispatch(addMessage({ userId, message: { id: "msg1", chatId: "chat1", date: 1000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false } }));

      const messages = [
        { id: "msg1", chatId: "chat1", date: 1000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false },
        { id: "msg2", chatId: "chat1", date: 2000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false },
      ];

      store.dispatch(addMessages({ userId, chatId: "chat1", messages }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.messagesOrder.chat1).toEqual(["msg1", "msg2"]);
    });

    it("updateMessage: updates fields of existing message", () => {
      const message = { id: "msg1", chatId: "chat1", date: 1000, fromId: "user1", message: "Original", isOutgoing: false, isEdited: false, isForwarded: false };
      store.dispatch(addMessage({ userId, message }));
      store.dispatch(updateMessage({ userId, chatId: "chat1", messageId: "msg1", updates: { message: "Updated" } }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.messages.chat1?.msg1.message).toBe("Updated");
      expect(state.messages.chat1?.msg1.fromId).toBe("user1");
    });

    it("updateMessage: does nothing if message does not exist", () => {
      store.dispatch(updateMessage({ userId, chatId: "chat1", messageId: "nonexistent", updates: { message: "Test" } }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.messages.chat1?.nonexistent).toBeUndefined();
    });

    it("removeMessage: removes from messages and messagesOrder", () => {
      store.dispatch(addMessage({ userId, message: { id: "msg1", chatId: "chat1", date: 1000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false } }));
      store.dispatch(addMessage({ userId, message: { id: "msg2", chatId: "chat1", date: 2000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false } }));
      store.dispatch(removeMessage({ userId, chatId: "chat1", messageId: "msg1" }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.messages.chat1?.msg1).toBeUndefined();
      expect(state.messagesOrder.chat1).toEqual(["msg2"]);
    });

    it("clearMessages: clears all messages for a chat", () => {
      store.dispatch(addMessage({ userId, message: { id: "msg1", chatId: "chat1", date: 1000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false } }));
      store.dispatch(addMessage({ userId, message: { id: "msg2", chatId: "chat1", date: 2000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false } }));
      store.dispatch(clearMessages({ userId, chatId: "chat1" }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.messages.chat1).toBeUndefined();
      expect(state.messagesOrder.chat1).toBeUndefined();
    });

    it("deleteChatMessages: removes specific messages by ID", () => {
      store.dispatch(addMessage({ userId, message: { id: "msg1", chatId: "chat1", date: 1000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false } }));
      store.dispatch(addMessage({ userId, message: { id: "msg2", chatId: "chat1", date: 2000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false } }));
      store.dispatch(addMessage({ userId, message: { id: "msg3", chatId: "chat1", date: 3000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false } }));

      store.dispatch(deleteChatMessages({ userId, chatId: "chat1", messageIds: ["msg1", "msg3"] }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.messages.chat1?.msg1).toBeUndefined();
      expect(state.messages.chat1?.msg2).toBeDefined();
      expect(state.messages.chat1?.msg3).toBeUndefined();
      expect(state.messagesOrder.chat1).toEqual(["msg2"]);
    });

    it("deleteChatMessages: cleans thread indices", () => {
      store.dispatch(addMessage({ userId, message: { id: "msg1", chatId: "chat1", date: 1000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false } }));
      store.dispatch(setThreadListedIds({ userId, chatId: "chat1", threadId: "0", listedIds: ["msg1"] }));
      store.dispatch(deleteChatMessages({ userId, chatId: "chat1", messageIds: ["msg1"] }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.threadIndex.chat1?.["0"]?.listedIds).toEqual([]);
    });

    it("addChatMessagesById: adds messages, sorts by date, syncs main thread index", () => {
      const messages = [
        { id: "msg3", chatId: "chat1", date: 3000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false },
        { id: "msg1", chatId: "chat1", date: 1000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false },
        { id: "msg2", chatId: "chat1", date: 2000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false },
      ];

      store.dispatch(addChatMessagesById({ userId, chatId: "chat1", messages }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.messagesOrder.chat1).toEqual(["msg1", "msg2", "msg3"]);
      expect(state.threadIndex.chat1?.["__main__"]?.listedIds).toEqual(["msg1", "msg2", "msg3"]);
    });

    it("addChatMessagesById: merges with existing messages", () => {
      store.dispatch(addMessage({ userId, message: { id: "msg1", chatId: "chat1", date: 1000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false } }));

      const messages = [
        { id: "msg2", chatId: "chat1", date: 2000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false },
        { id: "msg3", chatId: "chat1", date: 3000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false },
      ];

      store.dispatch(addChatMessagesById({ userId, chatId: "chat1", messages }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.messagesOrder.chat1).toEqual(["msg1", "msg2", "msg3"]);
    });
  });

  describe("Thread Management", () => {
    let store: ReturnType<typeof createStore>;

    beforeEach(() => {
      store = createStore();
      store.dispatch(addChat({ userId, chat: { id: "chat1", type: "private" as const, unreadCount: 0, isPinned: false } }));
    });

    it("addThread: adds thread to threads[chatId][threadId]", () => {
      const thread = {
        id: "1",
        chatId: "chat1",
        title: "Thread 1",
        messageCount: 0,
        unreadCount: 0,
        isPinned: false
      };

      store.dispatch(addThread({ userId, thread }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.threads.chat1?.["1"]).toEqual(thread);
    });

    it("updateThread: updates existing thread fields", () => {
      const thread = {
        id: "1",
        chatId: "chat1",
        title: "Thread 1",
        messageCount: 0,
        unreadCount: 0,
        isPinned: false
      };
      store.dispatch(addThread({ userId, thread }));
      store.dispatch(updateThread({ userId, chatId: "chat1", threadId: "1", updates: { title: "Updated Thread" } }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.threads.chat1?.["1"]?.title).toBe("Updated Thread");
    });

    it("updateThread: does nothing if thread does not exist", () => {
      store.dispatch(updateThread({ userId, chatId: "chat1", threadId: "1", updates: { title: "Updated" } }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.threads.chat1?.["1"]).toBeUndefined();
    });

    it("setSelectedThread: sets selectedThreadId", () => {
      store.dispatch(setSelectedChat({ userId, chatId: "chat1" }));
      store.dispatch(setSelectedThread({ userId, threadId: "5" }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.selectedThreadId).toBe("5");
    });
  });

  describe("Connection and Auth", () => {
    let store: ReturnType<typeof createStore>;

    beforeEach(() => {
      store = createStore();
    });

    it("setConnectionStatus: updates status and clears error on non-error", () => {
      store.dispatch(setConnectionError({ userId, error: "Previous error" }));
      store.dispatch(setConnectionStatus({ userId, status: "connected" }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.connectionStatus).toBe("connected");
      expect(state.connectionError).toBeNull();
    });

    it("setConnectionStatus: keeps error if status is error", () => {
      store.dispatch(setConnectionError({ userId, error: "Connection failed" }));
      store.dispatch(setConnectionStatus({ userId, status: "error" }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.connectionStatus).toBe("error");
      expect(state.connectionError).toBe("Connection failed");
    });

    it("setConnectionError: sets error and status to error", () => {
      store.dispatch(setConnectionStatus({ userId, status: "connected" }));
      store.dispatch(setConnectionError({ userId, error: "Network failure" }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.connectionError).toBe("Network failure");
      expect(state.connectionStatus).toBe("error");
    });

    it("setAuthStatus: updates auth status", () => {
      store.dispatch(setAuthStatus({ userId, status: "authenticated" }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.authStatus).toBe("authenticated");
    });

    it("setAuthError: sets auth error and status to error", () => {
      store.dispatch(setAuthStatus({ userId, status: "not_authenticated" }));
      store.dispatch(setAuthError({ userId, error: "Invalid credentials" }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.authError).toBe("Invalid credentials");
      expect(state.authStatus).toBe("error");
    });

    it("setPhoneNumber: updates phone number", () => {
      store.dispatch(setPhoneNumber({ userId, phoneNumber: "+1234567890" }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.phoneNumber).toBe("+1234567890");
    });

    it("setSessionString: updates session string", () => {
      store.dispatch(setSessionString({ userId, sessionString: "session123" }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.sessionString).toBe("session123");
    });

    it("setCurrentUser: updates current user", () => {
      const user = { id: "u1", firstName: "John", lastName: "Doe", isBot: false };
      store.dispatch(setCurrentUser({ userId, user }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.currentUser).toEqual(user);
    });
  });

  describe("Sync Status", () => {
    let store: ReturnType<typeof createStore>;

    beforeEach(() => {
      store = createStore();
    });

    it("setSyncStatus: updates isSyncing and isSynced", () => {
      store.dispatch(setSyncStatus({ userId, isSyncing: true, isSynced: false }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.isSyncing).toBe(true);
      expect(state.isSynced).toBe(false);
    });

    it("setSyncStatus: can update only isSyncing", () => {
      store.dispatch(setSyncStatus({ userId, isSyncing: true }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.isSyncing).toBe(true);
      expect(state.isSynced).toBe(false);
    });

    it("setSyncStatus: can update only isSynced", () => {
      store.dispatch(setSyncStatus({ userId, isSynced: true }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.isSyncing).toBe(false);
      expect(state.isSynced).toBe(true);
    });
  });

  describe("Sequencing", () => {
    let store: ReturnType<typeof createStore>;

    beforeEach(() => {
      store = createStore();
    });

    it("setCommonBoxState: sets seq/pts/qts/date", () => {
      store.dispatch(setCommonBoxState({
        userId,
        commonBoxState: { seq: 100, pts: 200, qts: 50, date: 1234567890 }
      }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.commonBoxState).toEqual({
        seq: 100,
        pts: 200,
        qts: 50,
        date: 1234567890,
      });
    });

    it("setCommonBoxState: replaces entire state", () => {
      store.dispatch(setCommonBoxState({
        userId,
        commonBoxState: { seq: 100, pts: 200, qts: 0, date: 0 }
      }));
      store.dispatch(setCommonBoxState({
        userId,
        commonBoxState: { seq: 150, pts: 250, qts: 50, date: 1234567890 }
      }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.commonBoxState).toEqual({
        seq: 150,
        pts: 250,
        qts: 50,
        date: 1234567890,
      });
    });

    it("setChannelPts: sets per-channel pts", () => {
      store.dispatch(setChannelPts({ userId, channelId: "channel1", pts: 150 }));
      store.dispatch(setChannelPts({ userId, channelId: "channel2", pts: 250 }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.channelPtsById.channel1).toBe(150);
      expect(state.channelPtsById.channel2).toBe(250);
    });
  });

  describe("Reset Functions", () => {
    let store: ReturnType<typeof createStore>;

    beforeEach(() => {
      store = createStore();
      store.dispatch(addChat({ userId, chat: { id: "chat1", type: "private" as const, unreadCount: 0, isPinned: false } }));
      store.dispatch(addMessage({ userId, message: { id: "msg1", chatId: "chat1", date: 1000, fromId: "user1", message: "", isOutgoing: false, isEdited: false, isForwarded: false } }));
      store.dispatch(setConnectionStatus({ userId, status: "connected" }));
      store.dispatch(setAuthStatus({ userId, status: "authenticated" }));
    });

    it("resetTelegramForUser: resets user state to initial", () => {
      store.dispatch(resetTelegramForUser({ userId }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.chats).toEqual({});
      expect(state.chatsOrder).toEqual([]);
      expect(state.messages).toEqual({});
      expect(state.messagesOrder).toEqual({});
      expect(state.connectionStatus).toBe("disconnected");
      expect(state.authStatus).toBe("not_authenticated");
    });

    it("resetChats: clears chats and order", () => {
      store.dispatch(resetChats({ userId }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.chats).toEqual({});
      expect(state.chatsOrder).toEqual([]);
      expect(state.messages).toBeDefined();
    });

    it("resetMessages: clears all messages", () => {
      store.dispatch(resetMessages({ userId }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.messages).toEqual({});
      expect(state.messagesOrder).toEqual({});
      expect(state.chats).toBeDefined();
    });
  });

  describe("Thread Indexing", () => {
    let store: ReturnType<typeof createStore>;

    beforeEach(() => {
      store = createStore();
      store.dispatch(addChat({ userId, chat: { id: "chat1", type: "private" as const, unreadCount: 0, isPinned: false } }));
    });

    it("setViewportIds: sets viewport IDs for chat+thread", () => {
      store.dispatch(setViewportIds({ userId, chatId: "chat1", threadId: "0", viewportIds: ["msg1", "msg2", "msg3"] }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.threadIndex.chat1?.["0"]?.viewportIds).toEqual(["msg1", "msg2", "msg3"]);
    });

    it("addOutlyingList: adds outlying list", () => {
      const ids = ["msg1", "msg2"];
      store.dispatch(addOutlyingList({ userId, chatId: "chat1", threadId: "0", ids }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.threadIndex.chat1?.["0"]?.outlyingLists).toContainEqual(ids);
    });

    it("addOutlyingList: appends to existing outlying lists", () => {
      const ids1 = ["msg1", "msg2"];
      const ids2 = ["msg3", "msg4"];

      store.dispatch(addOutlyingList({ userId, chatId: "chat1", threadId: "0", ids: ids1 }));
      store.dispatch(addOutlyingList({ userId, chatId: "chat1", threadId: "0", ids: ids2 }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.threadIndex.chat1?.["0"]?.outlyingLists).toHaveLength(2);
    });

    it("setThreadListedIds: sets listed IDs for a thread", () => {
      store.dispatch(setThreadListedIds({ userId, chatId: "chat1", threadId: "0", listedIds: ["msg1", "msg2", "msg3"] }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.threadIndex.chat1?.["0"]?.listedIds).toEqual(["msg1", "msg2", "msg3"]);
    });
  });

  describe("User Management", () => {
    let store: ReturnType<typeof createStore>;

    beforeEach(() => {
      store = createStore();
    });

    it("setUsers: replaces all users", () => {
      const users = {
        user1: { id: "user1", firstName: "Alice", isBot: false },
        user2: { id: "user2", firstName: "Bob", isBot: false },
      };

      store.dispatch(setUsers({ userId, users }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.users).toEqual(users);
    });

    it("addUsers: merges users with existing", () => {
      store.dispatch(setUsers({ userId, users: { user1: { id: "user1", firstName: "Alice", isBot: false } } }));
      store.dispatch(addUsers({ userId, users: { user2: { id: "user2", firstName: "Bob", isBot: false } } }));

      const state = store.getState().telegram.byUser[userId];
      expect(state.users.user1).toBeDefined();
      expect(state.users.user2).toBeDefined();
    });
  });

  describe("Multiple Users", () => {
    let store: ReturnType<typeof createStore>;

    beforeEach(() => {
      store = createStore();
    });

    it("maintains separate state for different users", () => {
      const userId1 = "u1";
      const userId2 = "u2";

      store.dispatch(addChat({ userId: userId1, chat: { id: "chat1", type: "private" as const, unreadCount: 0, isPinned: false } }));
      store.dispatch(addChat({ userId: userId2, chat: { id: "chat2", type: "group" as const, unreadCount: 0, isPinned: false } }));

      const state1 = store.getState().telegram.byUser[userId1];
      const state2 = store.getState().telegram.byUser[userId2];

      expect(state1.chats.chat1).toBeDefined();
      expect(state1.chats.chat2).toBeUndefined();
      expect(state2.chats.chat2).toBeDefined();
      expect(state2.chats.chat1).toBeUndefined();
    });

    it("resets only specified user state", () => {
      const userId1 = "u1";
      const userId2 = "u2";

      store.dispatch(addChat({ userId: userId1, chat: { id: "chat1", type: "private" as const, unreadCount: 0, isPinned: false } }));
      store.dispatch(addChat({ userId: userId2, chat: { id: "chat2", type: "group" as const, unreadCount: 0, isPinned: false } }));
      store.dispatch(resetTelegramForUser({ userId: userId1 }));

      const state1 = store.getState().telegram.byUser[userId1];
      const state2 = store.getState().telegram.byUser[userId2];

      expect(state1.chats).toEqual({});
      expect(state2.chats.chat2).toBeDefined();
    });
  });
});
