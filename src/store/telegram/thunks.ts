import { createAsyncThunk } from "@reduxjs/toolkit";
import { mtprotoService } from "../../services/mtprotoService";
import type { TelegramUser } from "./types";
import type { RootState } from "../index";

// Global flag to prevent concurrent checkAuthStatus calls
let isCheckingAuth = false;
let lastCheckTime = 0;
const MIN_CHECK_INTERVAL = 5000;

export const initializeTelegram = createAsyncThunk(
  "telegram/initialize",
  async (userId: string, { rejectWithValue }) => {
    try {
      await mtprotoService.initialize(userId);
      const sessionString = mtprotoService.getSessionString();
      return { sessionString };
    } catch (error) {
      return rejectWithValue(
        error instanceof Error
          ? error.message
          : "Failed to initialize Telegram client",
      );
    }
  },
);

export const connectTelegram = createAsyncThunk(
  "telegram/connect",
  async (_userId: string, { rejectWithValue }) => {
    if (!mtprotoService.isReady()) {
      return rejectWithValue(
        "MTProto client not initialized. Call initialize() first.",
      );
    }
    try {
      await mtprotoService.connect();
      return true;
    } catch (error) {
      return rejectWithValue(
        error instanceof Error
          ? error.message
          : "Failed to connect to Telegram",
      );
    }
  },
);

export const checkAuthStatus = createAsyncThunk(
  "telegram/checkAuthStatus",
  async (userId: string, { rejectWithValue, getState }) => {
    if (!mtprotoService.isReady()) {
      return rejectWithValue(
        "MTProto client not initialized. Call initialize() first.",
      );
    }

    const now = Date.now();
    if (isCheckingAuth && now - lastCheckTime < MIN_CHECK_INTERVAL) {
      const state = getState() as RootState;
      const u = state.telegram.byUser[userId];
      return (u?.currentUser as TelegramUser) || null;
    }

    isCheckingAuth = true;
    lastCheckTime = now;

    try {
      const client = mtprotoService.getClient();
      const isAuthorized = await client.checkAuthorization();

      if (!isAuthorized) {
        isCheckingAuth = false;
        return null;
      }

      try {
        const me = await mtprotoService.withFloodWaitHandling(async () => {
          return client.getMe();
        });
        isCheckingAuth = false;
        return me;
      } catch (error) {
        console.warn("getMe() failed, user not authenticated:", error);
        isCheckingAuth = false;
        return null;
      }
    } catch (error) {
      isCheckingAuth = false;
      if (
        error instanceof Error &&
        error.message.includes("AUTH_KEY_UNREGISTERED")
      ) {
        return null;
      }
      return rejectWithValue(
        error instanceof Error ? error.message : "Failed to check auth status",
      );
    }
  },
);

export const fetchChats = createAsyncThunk(
  "telegram/fetchChats",
  async (_userId: string, { rejectWithValue }) => {
    if (!mtprotoService.isReady()) {
      return rejectWithValue(
        "MTProto client not initialized. Call initialize() first.",
      );
    }
    try {
      const client = mtprotoService.getClient();
      const dialogs = await mtprotoService.withFloodWaitHandling(async () => {
        return client.getDialogs({ limit: 100 });
      });
      return dialogs;
    } catch (error) {
      return rejectWithValue(
        error instanceof Error ? error.message : "Failed to fetch chats",
      );
    }
  },
);

export const fetchMessages = createAsyncThunk(
  "telegram/fetchMessages",
  async (
    {
      userId: _userId,
      chatId,
      limit = 50,
      offsetId,
    }: { userId: string; chatId: string; limit?: number; offsetId?: number },
    { rejectWithValue },
  ) => {
    if (!mtprotoService.isReady()) {
      return rejectWithValue(
        "MTProto client not initialized. Call initialize() first.",
      );
    }
    try {
      void _userId;
      const client = mtprotoService.getClient();
      const messages = await mtprotoService.withFloodWaitHandling(async () => {
        return client.getMessages(chatId, { limit, offsetId });
      });
      return { chatId, messages };
    } catch (error) {
      return rejectWithValue(
        error instanceof Error ? error.message : "Failed to fetch messages",
      );
    }
  },
);
