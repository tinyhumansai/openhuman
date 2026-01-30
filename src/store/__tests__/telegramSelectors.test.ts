import { describe, it, expect } from "vitest";
import {
  selectTelegramUserState,
  selectIsConnected,
  selectIsAuthenticated,
  selectCurrentUser,
  selectOrderedChats,
  selectChatMessages,
  selectTotalUnreadCount,
  selectPinnedChats,
  selectChatMessageById,
  selectTelegramReady,
  selectConnectionStatus,
  selectAuthStatus,
  selectIsInitialized,
  selectSelectedChat,
  selectFilteredChats,
  selectChatLatestMessage,
} from "../telegramSelectors";
import type { TelegramState, TelegramChat, TelegramMessage } from "../telegram/types";
import { initialState } from "../telegram/types";
import type { RootState } from "../index";

/**
 * Helper to build a mock RootState with customizable telegram state
 */
function mockState(
  telegramOverrides: Partial<TelegramState> = {},
  userId = "u1"
): RootState {
  return {
    auth: { token: "tok", isOnboardedByUser: {} },
    socket: { byUser: {} },
    user: {
      user: { _id: userId, telegramId: 1 } as any,
      isLoading: false,
      error: null,
    },
    telegram: {
      byUser: {
        [userId]: { ...initialState, ...telegramOverrides },
      },
    },
  } as any;
}

describe("telegramSelectors", () => {
  describe("selectTelegramUserState", () => {
    it("returns user state when user exists", () => {
      const state = mockState({ connectionStatus: "connected" });
      const result = selectTelegramUserState(state);
      expect(result.connectionStatus).toBe("connected");
    });

    it("returns default initialState when user not found", () => {
      const state = mockState({}, "u1");
      // Clear the user from state
      state.user.user = null;
      const result = selectTelegramUserState(state);
      expect(result).toEqual(initialState);
    });

    it("returns default initialState when userId has no telegram state", () => {
      const state = mockState({}, "u1");
      state.telegram.byUser = {}; // Empty byUser map
      const result = selectTelegramUserState(state);
      expect(result).toEqual(initialState);
    });
  });

  describe("selectIsConnected", () => {
    it("returns true when connectionStatus is connected", () => {
      const state = mockState({ connectionStatus: "connected" });
      expect(selectIsConnected(state)).toBe(true);
    });

    it("returns false when connectionStatus is disconnected", () => {
      const state = mockState({ connectionStatus: "disconnected" });
      expect(selectIsConnected(state)).toBe(false);
    });

    it("returns false when connectionStatus is connecting", () => {
      const state = mockState({ connectionStatus: "connecting" });
      expect(selectIsConnected(state)).toBe(false);
    });

    it("returns false when connectionStatus is error", () => {
      const state = mockState({ connectionStatus: "error" });
      expect(selectIsConnected(state)).toBe(false);
    });
  });

  describe("selectIsAuthenticated", () => {
    it("returns true when authStatus is authenticated", () => {
      const state = mockState({ authStatus: "authenticated" });
      expect(selectIsAuthenticated(state)).toBe(true);
    });

    it("returns false when authStatus is not_authenticated", () => {
      const state = mockState({ authStatus: "not_authenticated" });
      expect(selectIsAuthenticated(state)).toBe(false);
    });

    it("returns false when authStatus is authenticating", () => {
      const state = mockState({ authStatus: "authenticating" });
      expect(selectIsAuthenticated(state)).toBe(false);
    });

    it("returns false when authStatus is error", () => {
      const state = mockState({ authStatus: "error" });
      expect(selectIsAuthenticated(state)).toBe(false);
    });
  });

  describe("selectCurrentUser", () => {
    it("returns currentUser from state", () => {
      const currentUser = {
        id: "123",
        firstName: "Alice",
        lastName: "Smith",
        username: "alice",
        isBot: false,
      };
      const state = mockState({ currentUser });
      expect(selectCurrentUser(state)).toEqual(currentUser);
    });

    it("returns null when currentUser is not set", () => {
      const state = mockState({ currentUser: null });
      expect(selectCurrentUser(state)).toBeNull();
    });
  });

  describe("selectOrderedChats", () => {
    it("returns chats in chatsOrder sequence", () => {
      const chat1: TelegramChat = {
        id: "c1",
        title: "Chat 1",
        type: "private",
        unreadCount: 0,
        isPinned: false,
      };
      const chat2: TelegramChat = {
        id: "c2",
        title: "Chat 2",
        type: "group",
        unreadCount: 5,
        isPinned: true,
      };
      const state = mockState({
        chats: { c1: chat1, c2: chat2 },
        chatsOrder: ["c2", "c1"],
      });
      const result = selectOrderedChats(state);
      expect(result).toEqual([chat2, chat1]);
    });

    it("skips missing chats that are in chatsOrder", () => {
      const chat1: TelegramChat = {
        id: "c1",
        title: "Chat 1",
        type: "private",
        unreadCount: 0,
        isPinned: false,
      };
      const state = mockState({
        chats: { c1: chat1 },
        chatsOrder: ["c1", "c2", "c3"], // c2, c3 missing
      });
      const result = selectOrderedChats(state);
      expect(result).toEqual([chat1]);
    });

    it("returns empty array when no chats", () => {
      const state = mockState({ chats: {}, chatsOrder: [] });
      const result = selectOrderedChats(state);
      expect(result).toEqual([]);
    });
  });

  describe("selectChatMessages", () => {
    it("returns ordered messages for a chat", () => {
      const msg1: TelegramMessage = {
        id: "m1",
        chatId: "c1",
        date: 1000,
        message: "Hello",
        isOutgoing: true,
        isEdited: false,
        isForwarded: false,
      };
      const msg2: TelegramMessage = {
        id: "m2",
        chatId: "c1",
        date: 2000,
        message: "World",
        isOutgoing: false,
        isEdited: false,
        isForwarded: false,
      };
      const state = mockState({
        messages: {
          c1: { m1: msg1, m2: msg2 },
        },
        messagesOrder: {
          c1: ["m1", "m2"],
        },
      });
      const result = selectChatMessages(state, "c1");
      expect(result).toEqual([msg1, msg2]);
    });

    it("returns empty array when chat has no messages", () => {
      const state = mockState({
        messages: {},
        messagesOrder: {},
      });
      const result = selectChatMessages(state, "c1");
      expect(result).toEqual([]);
    });

    it("skips missing messages in messagesOrder", () => {
      const msg1: TelegramMessage = {
        id: "m1",
        chatId: "c1",
        date: 1000,
        message: "Hello",
        isOutgoing: true,
        isEdited: false,
        isForwarded: false,
      };
      const state = mockState({
        messages: {
          c1: { m1: msg1 },
        },
        messagesOrder: {
          c1: ["m1", "m2"], // m2 missing
        },
      });
      const result = selectChatMessages(state, "c1");
      expect(result).toEqual([msg1]);
    });
  });

  describe("selectTotalUnreadCount", () => {
    it("sums unreadCount across all ordered chats", () => {
      const chat1: TelegramChat = {
        id: "c1",
        title: "Chat 1",
        type: "private",
        unreadCount: 5,
        isPinned: false,
      };
      const chat2: TelegramChat = {
        id: "c2",
        title: "Chat 2",
        type: "group",
        unreadCount: 10,
        isPinned: false,
      };
      const chat3: TelegramChat = {
        id: "c3",
        title: "Chat 3",
        type: "channel",
        unreadCount: 0,
        isPinned: true,
      };
      const state = mockState({
        chats: { c1: chat1, c2: chat2, c3: chat3 },
        chatsOrder: ["c1", "c2", "c3"],
      });
      expect(selectTotalUnreadCount(state)).toBe(15);
    });

    it("returns 0 when no chats have unread messages", () => {
      const chat1: TelegramChat = {
        id: "c1",
        title: "Chat 1",
        type: "private",
        unreadCount: 0,
        isPinned: false,
      };
      const state = mockState({
        chats: { c1: chat1 },
        chatsOrder: ["c1"],
      });
      expect(selectTotalUnreadCount(state)).toBe(0);
    });

    it("returns 0 when no chats", () => {
      const state = mockState({
        chats: {},
        chatsOrder: [],
      });
      expect(selectTotalUnreadCount(state)).toBe(0);
    });
  });

  describe("selectPinnedChats", () => {
    it("filters to only pinned chats", () => {
      const chat1: TelegramChat = {
        id: "c1",
        title: "Chat 1",
        type: "private",
        unreadCount: 0,
        isPinned: false,
      };
      const chat2: TelegramChat = {
        id: "c2",
        title: "Chat 2",
        type: "group",
        unreadCount: 5,
        isPinned: true,
      };
      const chat3: TelegramChat = {
        id: "c3",
        title: "Chat 3",
        type: "channel",
        unreadCount: 0,
        isPinned: true,
      };
      const state = mockState({
        chats: { c1: chat1, c2: chat2, c3: chat3 },
        chatsOrder: ["c1", "c2", "c3"],
      });
      const result = selectPinnedChats(state);
      expect(result).toEqual([chat2, chat3]);
    });

    it("returns empty array when no pinned chats", () => {
      const chat1: TelegramChat = {
        id: "c1",
        title: "Chat 1",
        type: "private",
        unreadCount: 0,
        isPinned: false,
      };
      const state = mockState({
        chats: { c1: chat1 },
        chatsOrder: ["c1"],
      });
      const result = selectPinnedChats(state);
      expect(result).toEqual([]);
    });
  });

  describe("selectChatMessageById", () => {
    it("performs O(1) lookup by chatId and messageId", () => {
      const msg1: TelegramMessage = {
        id: "m1",
        chatId: "c1",
        date: 1000,
        message: "Hello",
        isOutgoing: true,
        isEdited: false,
        isForwarded: false,
      };
      const state = mockState({
        messages: {
          c1: { m1: msg1 },
        },
      });
      const result = selectChatMessageById(state, "c1", "m1");
      expect(result).toEqual(msg1);
    });

    it("returns undefined when message not found", () => {
      const state = mockState({
        messages: {
          c1: {},
        },
      });
      const result = selectChatMessageById(state, "c1", "m1");
      expect(result).toBeUndefined();
    });

    it("returns undefined when chat not found", () => {
      const state = mockState({
        messages: {},
      });
      const result = selectChatMessageById(state, "c1", "m1");
      expect(result).toBeUndefined();
    });
  });

  describe("selectTelegramReady", () => {
    it("returns true when connected, authenticated, and initialized", () => {
      const state = mockState({
        connectionStatus: "connected",
        authStatus: "authenticated",
        isInitialized: true,
      });
      expect(selectTelegramReady(state)).toBe(true);
    });

    it("returns false when not connected", () => {
      const state = mockState({
        connectionStatus: "disconnected",
        authStatus: "authenticated",
        isInitialized: true,
      });
      expect(selectTelegramReady(state)).toBe(false);
    });

    it("returns false when not authenticated", () => {
      const state = mockState({
        connectionStatus: "connected",
        authStatus: "not_authenticated",
        isInitialized: true,
      });
      expect(selectTelegramReady(state)).toBe(false);
    });

    it("returns false when not initialized", () => {
      const state = mockState({
        connectionStatus: "connected",
        authStatus: "authenticated",
        isInitialized: false,
      });
      expect(selectTelegramReady(state)).toBe(false);
    });

    it("returns false when none are ready", () => {
      const state = mockState({
        connectionStatus: "disconnected",
        authStatus: "not_authenticated",
        isInitialized: false,
      });
      expect(selectTelegramReady(state)).toBe(false);
    });
  });

  describe("additional selectors", () => {
    it("selectConnectionStatus returns connection status", () => {
      const state = mockState({ connectionStatus: "connecting" });
      expect(selectConnectionStatus(state)).toBe("connecting");
    });

    it("selectAuthStatus returns auth status", () => {
      const state = mockState({ authStatus: "authenticating" });
      expect(selectAuthStatus(state)).toBe("authenticating");
    });

    it("selectIsInitialized returns initialization status", () => {
      const state = mockState({ isInitialized: true });
      expect(selectIsInitialized(state)).toBe(true);
    });

    it("selectSelectedChat returns selected chat", () => {
      const chat1: TelegramChat = {
        id: "c1",
        title: "Chat 1",
        type: "private",
        unreadCount: 0,
        isPinned: false,
      };
      const state = mockState({
        chats: { c1: chat1 },
        selectedChatId: "c1",
      });
      expect(selectSelectedChat(state)).toEqual(chat1);
    });

    it("selectSelectedChat returns null when no chat selected", () => {
      const state = mockState({ selectedChatId: null });
      expect(selectSelectedChat(state)).toBeNull();
    });

    it("selectFilteredChats filters by filteredChatIds", () => {
      const chat1: TelegramChat = {
        id: "c1",
        title: "Chat 1",
        type: "private",
        unreadCount: 0,
        isPinned: false,
      };
      const chat2: TelegramChat = {
        id: "c2",
        title: "Chat 2",
        type: "group",
        unreadCount: 5,
        isPinned: true,
      };
      const state = mockState({
        chats: { c1: chat1, c2: chat2 },
        chatsOrder: ["c1", "c2"],
        filteredChatIds: ["c2"],
      });
      const result = selectFilteredChats(state);
      expect(result).toEqual([chat2]);
    });

    it("selectFilteredChats returns all chats when no filter", () => {
      const chat1: TelegramChat = {
        id: "c1",
        title: "Chat 1",
        type: "private",
        unreadCount: 0,
        isPinned: false,
      };
      const state = mockState({
        chats: { c1: chat1 },
        chatsOrder: ["c1"],
        filteredChatIds: null,
      });
      const result = selectFilteredChats(state);
      expect(result).toEqual([chat1]);
    });

    it("selectChatLatestMessage returns last message", () => {
      const msg1: TelegramMessage = {
        id: "m1",
        chatId: "c1",
        date: 1000,
        message: "Hello",
        isOutgoing: true,
        isEdited: false,
        isForwarded: false,
      };
      const msg2: TelegramMessage = {
        id: "m2",
        chatId: "c1",
        date: 2000,
        message: "World",
        isOutgoing: false,
        isEdited: false,
        isForwarded: false,
      };
      const state = mockState({
        messages: {
          c1: { m1: msg1, m2: msg2 },
        },
        messagesOrder: {
          c1: ["m1", "m2"],
        },
      });
      const result = selectChatLatestMessage(state, "c1");
      expect(result).toEqual(msg2);
    });

    it("selectChatLatestMessage returns null when no messages", () => {
      const state = mockState({
        messages: {},
        messagesOrder: {},
      });
      const result = selectChatLatestMessage(state, "c1");
      expect(result).toBeNull();
    });
  });
});
