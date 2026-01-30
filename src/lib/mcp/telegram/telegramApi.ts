/**
 * Telegram API helpers for MCP tools
 * Uses mtprotoService + Redux telegram state (alphahuman)
 *
 * "WithApiFallback" variants try cached Redux state first, then call the
 * Telegram API when cache is empty. They call enforceRateLimit() internally
 * before any API call so the fast cache path stays unthrottled.
 */

import { store } from "../../../store";
import { mtprotoService } from "../../../services/mtprotoService";
import {
  selectOrderedChats,
  selectCurrentUser,
  selectTelegramUserState,
} from "../../../store/telegramSelectors";
import type {
  TelegramChat,
  TelegramUser,
  TelegramMessage,
} from "../../../store/telegram/types";
import { Api } from "telegram";
import bigInt from "big-integer";
import { enforceRateLimit } from "../rateLimiter";

function getTelegramState() {
  return selectTelegramUserState(store.getState());
}

export interface FormattedEntity {
  id: string;
  name: string;
  type: string;
  username?: string;
  phone?: string;
}

export interface FormattedMessage {
  id: number | string;
  date: string;
  text: string;
  from_id?: string;
  has_media?: boolean;
  media_type?: string;
}

/**
 * Get chat by ID or username
 */
export function getChatById(chatId: string | number): TelegramChat | undefined {
  const state = getTelegramState();
  const idStr = String(chatId);

  const chat = state.chats[idStr];
  if (chat) return chat;

  if (
    typeof chatId === "string" &&
    (chatId.startsWith("@") || /^[a-zA-Z0-9_]+$/.test(chatId))
  ) {
    const username = chatId.startsWith("@") ? chatId : `@${chatId}`;
    return Object.values(state.chats).find(
      (c) =>
        c.username &&
        (c.username === username || c.username === username.slice(1)),
    );
  }

  return undefined;
}

/**
 * Get user by ID (current user only for now; no full user cache)
 */
export function getUserById(userId: string | number): TelegramUser | undefined {
  const state = getTelegramState();
  const current = state.currentUser;
  if (!current) return undefined;
  if (String(current.id) === String(userId)) return current;
  return undefined;
}

/**
 * Get messages from a chat (from store cache)
 * @param offset - numeric offset for pagination (default 0)
 */
export async function getMessages(
  chatId: string | number,
  limit = 20,
  offset = 0,
): Promise<TelegramMessage[] | undefined> {
  const chat = getChatById(chatId);
  if (!chat) return undefined;

  const state = getTelegramState();
  const order = state.messagesOrder[chat.id] ?? [];
  const byId = state.messages[chat.id] ?? {};
  const all = order.map((id) => byId[id]).filter(Boolean);
  const list = all.slice(offset, offset + limit);

  return list.length ? list : undefined;
}

/**
 * Send a message to a chat
 */
export async function sendMessage(
  chatId: string | number,
  message: string,
  replyToMessageId?: number,
): Promise<{ id: string } | undefined> {
  const chat = getChatById(chatId);
  if (!chat) return undefined;

  const entity = chat.username ? `@${chat.username.replace("@", "")}` : chat.id;

  if (replyToMessageId !== undefined) {
    const client = mtprotoService.getClient();
    await mtprotoService.withFloodWaitHandling(async () => {
      await client.sendMessage(entity, {
        message,
        replyTo: replyToMessageId,
      });
    });
  } else {
    await mtprotoService.sendMessage(entity, message);
  }

  return { id: String(Date.now()) };
}

/**
 * Get list of chats (from store)
 */
export async function getChats(limit = 20): Promise<TelegramChat[]> {
  const state = store.getState();
  const ordered = selectOrderedChats(state);
  return ordered.slice(0, limit);
}

/**
 * Search chats by query (filter by title/username from store)
 */
export async function searchChats(query: string): Promise<TelegramChat[]> {
  const state = store.getState();
  const ordered = selectOrderedChats(state);
  const q = query.toLowerCase();
  return ordered.filter((c) => {
    const title = (c.title ?? "").toLowerCase();
    const un = (c.username ?? "").toLowerCase();
    return title.includes(q) || un.includes(q);
  });
}

/**
 * Get current user info
 */
export function getCurrentUser(): TelegramUser | undefined {
  const state = store.getState();
  return selectCurrentUser(state) ?? undefined;
}

/**
 * Format entity (chat or user) for display
 */
export function formatEntity(
  entity: TelegramChat | TelegramUser,
): FormattedEntity {
  if ("title" in entity) {
    const chat = entity as TelegramChat;
    const type =
      chat.type === "channel"
        ? "channel"
        : chat.type === "supergroup"
          ? "group"
          : chat.type;
    return {
      id: chat.id,
      name: chat.title ?? "Unknown",
      type,
      username: chat.username,
    };
  }
  const user = entity as TelegramUser;
  const name =
    [user.firstName, user.lastName].filter(Boolean).join(" ") || "Unknown";
  return {
    id: user.id,
    name,
    type: "user",
    username: user.username,
    phone: user.phoneNumber,
  };
}

/**
 * Format message for display
 */
export function formatMessage(message: TelegramMessage): FormattedMessage {
  const result: FormattedMessage = {
    id: message.id,
    date: new Date(message.date * 1000).toISOString(),
    text: message.message ?? "",
  };
  if (message.fromId) result.from_id = message.fromId;
  if (message.media?.type) {
    result.has_media = true;
    result.media_type = message.media.type;
  }
  return result;
}

// ---------------------------------------------------------------------------
// API Fallback helpers
// ---------------------------------------------------------------------------

/** Convert a raw GramJS message to our TelegramMessage format */
function apiMessageToTelegramMessage(
  msg: Record<string, unknown>,
  chatId: string,
): TelegramMessage {
  const fromId =
    msg.fromId && typeof msg.fromId === "object" && "userId" in (msg.fromId as object)
      ? String((msg.fromId as { userId: unknown }).userId)
      : undefined;

  const replyTo = msg.replyTo as { replyToMsgId?: number } | undefined;

  const media = msg.media as { className?: string } | undefined;
  let mediaInfo: TelegramMessage["media"] | undefined;
  if (media && media.className && media.className !== "MessageMediaEmpty") {
    mediaInfo = { type: media.className };
  }

  return {
    id: String(msg.id ?? ""),
    chatId,
    date: typeof msg.date === "number" ? msg.date : 0,
    message: typeof msg.message === "string" ? msg.message : "",
    fromId,
    isOutgoing: Boolean(msg.out),
    isEdited: msg.editDate != null,
    isForwarded: msg.fwdFrom != null,
    replyToMessageId: replyTo?.replyToMsgId != null
      ? String(replyTo.replyToMsgId)
      : undefined,
    media: mediaInfo,
  };
}

/**
 * Get messages — tries cached Redux state first, falls back to Telegram API.
 * Calls enforceRateLimit() internally before any API call.
 */
export async function getMessagesWithApiFallback(
  chatId: string | number,
  limit = 20,
  offset = 0,
): Promise<TelegramMessage[] | undefined> {
  // 1. Try cache
  const cached = await getMessages(chatId, limit, offset);
  if (cached && cached.length > 0) return cached;

  // 2. Resolve chat for API call
  const chat = getChatById(chatId);
  if (!chat) return undefined;

  // 3. Rate limit before API call
  try {
    await enforceRateLimit("__api_fallback_messages");
  } catch {
    return undefined; // Rate limited — return nothing rather than error
  }

  // 4. Fetch from Telegram API
  try {
    const client = mtprotoService.getClient();
    const entity = chat.username ? chat.username : chat.id;
    const inputPeer = await client.getInputEntity(entity);

    const result = await mtprotoService.withFloodWaitHandling(async () => {
      return client.invoke(
        new Api.messages.GetHistory({
          peer: inputPeer,
          offsetId: 0,
          offsetDate: 0,
          addOffset: offset,
          limit,
          maxId: 0,
          minId: 0,
          hash: bigInt(0),
        }),
      );
    });

    if ("messages" in result && Array.isArray(result.messages)) {
      const messages = (result.messages as unknown as Record<string, unknown>[])
        .filter((m) => m.className !== "MessageEmpty")
        .map((m) => apiMessageToTelegramMessage(m, chat.id));

      // Enrich fromName from users list
      if ("users" in result && Array.isArray(result.users)) {
        const usersById = new Map<string, string>();
        for (const u of result.users as Array<{
          id?: unknown;
          firstName?: string;
          lastName?: string;
        }>) {
          if (u.id != null) {
            const name =
              [u.firstName, u.lastName].filter(Boolean).join(" ") || "Unknown";
            usersById.set(String(u.id), name);
          }
        }
        for (const msg of messages) {
          if (msg.fromId && !msg.fromName) {
            msg.fromName = usersById.get(msg.fromId);
          }
        }
      }

      return messages.length > 0 ? messages : undefined;
    }
  } catch {
    // API failed — return nothing
  }

  return undefined;
}

/** Convert a raw GramJS dialog + chat/user to our TelegramChat format */
function apiDialogToTelegramChat(
  dialog: Record<string, unknown>,
  chatsById: Map<string, Record<string, unknown>>,
  usersById: Map<string, Record<string, unknown>>,
): TelegramChat | undefined {
  const peer = dialog.peer as { className?: string; userId?: unknown; chatId?: unknown; channelId?: unknown } | undefined;
  if (!peer) return undefined;

  let id: string;
  let type: TelegramChat["type"];
  let raw: Record<string, unknown> | undefined;

  if (peer.className === "PeerUser" && peer.userId != null) {
    id = String(peer.userId);
    type = "private";
    raw = usersById.get(id);
  } else if (peer.className === "PeerChat" && peer.chatId != null) {
    id = String(peer.chatId);
    type = "group";
    raw = chatsById.get(id);
  } else if (peer.className === "PeerChannel" && peer.channelId != null) {
    id = String(peer.channelId);
    raw = chatsById.get(id);
    type = raw && Boolean(raw.megagroup) ? "supergroup" : "channel";
  } else {
    return undefined;
  }

  let title: string;
  let username: string | undefined;
  if (raw) {
    title =
      (raw.title as string) ??
      [raw.firstName, raw.lastName].filter(Boolean).join(" ") ??
      "Unknown";
    username = raw.username as string | undefined;
  } else {
    title = "Unknown";
  }

  return {
    id,
    title,
    type,
    username,
    unreadCount: typeof dialog.unreadCount === "number" ? dialog.unreadCount : 0,
    isPinned: Boolean(dialog.pinned),
  };
}

/**
 * Get chats — tries cached Redux state first, falls back to Telegram API.
 * Calls enforceRateLimit() internally before any API call.
 */
export async function getChatsWithApiFallback(
  limit = 20,
): Promise<TelegramChat[]> {
  // 1. Try cache
  const cached = await getChats(limit);
  if (cached.length > 0) return cached;

  // 2. Rate limit before API call
  try {
    await enforceRateLimit("__api_fallback_chats");
  } catch {
    return [];
  }

  // 3. Fetch from Telegram API
  try {
    const client = mtprotoService.getClient();

    const result = await mtprotoService.withFloodWaitHandling(async () => {
      return client.invoke(
        new Api.messages.GetDialogs({
          offsetDate: 0,
          offsetId: 0,
          offsetPeer: new Api.InputPeerEmpty(),
          limit,
          hash: bigInt(0),
        }),
      );
    });

    if (!("dialogs" in result) || !Array.isArray(result.dialogs)) return [];

    // Index chats and users by ID
    const chatsById = new Map<string, Record<string, unknown>>();
    const usersById = new Map<string, Record<string, unknown>>();
    if ("chats" in result && Array.isArray(result.chats)) {
      for (const c of result.chats as unknown as Record<string, unknown>[]) {
        if (c.id != null) chatsById.set(String(c.id), c);
      }
    }
    if ("users" in result && Array.isArray(result.users)) {
      for (const u of result.users as unknown as Record<string, unknown>[]) {
        if (u.id != null) usersById.set(String(u.id), u);
      }
    }

    return (result.dialogs as unknown as Record<string, unknown>[])
      .map((d) => apiDialogToTelegramChat(d, chatsById, usersById))
      .filter((c): c is TelegramChat => c !== undefined)
      .slice(0, limit);
  } catch {
    return [];
  }
}

/**
 * Get current user — tries cached Redux state first, falls back to client.getMe().
 * Calls enforceRateLimit() internally before any API call.
 */
export async function getCurrentUserWithApiFallback(): Promise<
  TelegramUser | undefined
> {
  // 1. Try cache
  const cached = getCurrentUser();
  if (cached) return cached;

  // 2. Rate limit before API call
  try {
    await enforceRateLimit("__api_fallback_me");
  } catch {
    return undefined;
  }

  // 3. Fetch from Telegram API
  try {
    const client = mtprotoService.getClient();
    const me = await mtprotoService.withFloodWaitHandling(async () => {
      return client.getMe();
    });

    if (!me) return undefined;

    const raw = me as unknown as {
      id?: unknown;
      firstName?: string;
      lastName?: string;
      username?: string;
      phone?: string;
      bot?: boolean;
    };

    return {
      id: String(raw.id ?? ""),
      firstName: raw.firstName ?? "",
      lastName: raw.lastName,
      username: raw.username,
      phoneNumber: raw.phone,
      isBot: Boolean(raw.bot),
    };
  } catch {
    return undefined;
  }
}
