// Types for Telegram entities
export type TelegramConnectionStatus =
  | "disconnected"
  | "connecting"
  | "connected"
  | "error";
export type TelegramAuthStatus =
  | "not_authenticated"
  | "authenticating"
  | "authenticated"
  | "error";

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
  type: "private" | "group" | "supergroup" | "channel";
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

/**
 * Per-thread message indexing state.
 * Tracks which messages are loaded, visible, and where gaps exist.
 */
export interface ThreadMessageState {
  /** All loaded message IDs in chronological order (contiguous range) */
  listedIds: string[];
  /** Disjoint loaded ranges when there are gaps in history (e.g. user jumped to a pinned message) */
  outlyingLists: string[][];
  /** Currently visible message IDs (capped at viewport limit) */
  viewportIds: string[];
}

/** Default key for the main (non-threaded) conversation */
export const MAIN_THREAD_ID = "__main__";

export interface TelegramState {
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

  // ---------------------------------------------------------------------------
  // Messages — Normalized storage
  // ---------------------------------------------------------------------------

  /** Flat message map per chat: messages[chatId][messageId] = message (O(1) lookup) */
  messages: Record<string, Record<string, TelegramMessage>>;

  /** Ordered message IDs per chat — equivalent of main-thread listedIds */
  messagesOrder: Record<string, string[]>;

  /** Per-chat, per-thread message indexing (listedIds, outlyingLists, viewportIds) */
  threadIndex: Record<string, Record<string, ThreadMessageState>>;

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

  // ---------------------------------------------------------------------------
  // Update sequencing state (pts/qts/seq)
  // ---------------------------------------------------------------------------

  /** Common update box state — global pts/qts/seq/date */
  commonBoxState: {
    seq: number;
    date: number;
    pts: number;
    qts: number;
  };

  /** Per-channel PTS tracking */
  channelPtsById: Record<string, number>;
}

export const initialState: TelegramState = {
  // Connection
  connectionStatus: "disconnected",
  connectionError: null,

  // Authentication
  authStatus: "not_authenticated",
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
  threadIndex: {},

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

  // Update sequencing
  commonBoxState: { seq: 0, date: 0, pts: 0, qts: 0 },
  channelPtsById: {},
};

/** Root telegram slice state: per-user */
export interface TelegramRootState {
  byUser: Record<string, TelegramState>;
}
