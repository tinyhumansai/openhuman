/**
 * Smart Message Loading Service
 *
 * Implements Telegram-TT–style message fetching with:
 * - Three-direction loading (Around, Backward, Forward)
 * - AbortController per chat/thread for request cancellation
 * - Batch ID fetching (up to 100 IDs per request)
 * - Budget preloading (two-pass: viewport first, buffer second)
 *
 * Reference: Telegram-TT's src/global/actions/api/messages.ts
 */

import { Api } from "telegram/tl";
import { mtprotoService } from "./mtprotoService";
import { store } from "../store";
import {
  addChatMessagesById,
  setViewportIds,
  addOutlyingList,
} from "../store/telegram";
import { selectTelegramCurrentUserId } from "../store/telegramSelectors";
import type { TelegramMessage } from "../store/telegram/types";
import { MAIN_THREAD_ID } from "../store/telegram/types";

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/** Number of messages per fetch slice */
const MESSAGE_LIST_SLICE_DESKTOP = 60;
const MESSAGE_LIST_SLICE_MOBILE = 40;

/** Viewport cap = slice × 2 */
const VIEWPORT_LIMIT_DESKTOP = MESSAGE_LIST_SLICE_DESKTOP * 2;
const VIEWPORT_LIMIT_MOBILE = MESSAGE_LIST_SLICE_MOBILE * 2;

/** Max IDs per batch fetch (Telegram API limit) */
const BATCH_FETCH_LIMIT = 100;

/** Detect mobile viewport */
function isMobile(): boolean {
  return typeof window !== "undefined" && window.innerWidth < 768;
}

function getSliceSize(): number {
  return isMobile() ? MESSAGE_LIST_SLICE_MOBILE : MESSAGE_LIST_SLICE_DESKTOP;
}

function getViewportLimit(): number {
  return isMobile() ? VIEWPORT_LIMIT_MOBILE : VIEWPORT_LIMIT_DESKTOP;
}

// ---------------------------------------------------------------------------
// Loading direction
// ---------------------------------------------------------------------------

export type LoadDirection = "around" | "backward" | "forward";

export interface LoadMessagesParams {
  chatId: string;
  direction: LoadDirection;
  /** Target message ID for 'around' loading, or edge message ID for directional */
  offsetId?: number;
  threadId?: string;
  /** If true, this is a buffer preload (second pass) */
  isBudgetPreload?: boolean;
}

// ---------------------------------------------------------------------------
// AbortController management (per chat + thread)
// ---------------------------------------------------------------------------

const abortControllers = new Map<string, AbortController>();

function getAbortKey(chatId: string, threadId?: string): string {
  return threadId ? `${chatId}:${threadId}` : chatId;
}

/**
 * Cancel any in-flight message fetch for this chat/thread.
 * Call when user switches chats to prevent stale data.
 */
export function cancelMessageFetch(
  chatId: string,
  threadId?: string,
): void {
  const key = getAbortKey(chatId, threadId);
  const existing = abortControllers.get(key);
  if (existing) {
    existing.abort();
    abortControllers.delete(key);
  }
}

function getOrCreateAbortController(
  chatId: string,
  threadId?: string,
): AbortController {
  const key = getAbortKey(chatId, threadId);
  // Cancel previous if still active
  cancelMessageFetch(chatId, threadId);
  const controller = new AbortController();
  abortControllers.set(key, controller);
  return controller;
}

// ---------------------------------------------------------------------------
// Core loading function
// ---------------------------------------------------------------------------

/**
 * Load messages for a chat with proper direction handling.
 *
 * - **around**: Centers viewport on `offsetId` (initial load / jump-to-message)
 * - **backward**: Loads older messages from `offsetId`
 * - **forward**: Loads newer messages from `offsetId`
 */
export async function loadMessages(
  params: LoadMessagesParams,
): Promise<TelegramMessage[]> {
  const { chatId, direction, offsetId, threadId, isBudgetPreload } = params;
  const slice = getSliceSize();
  const controller = getOrCreateAbortController(chatId, threadId);

  if (!mtprotoService.isReady() || !mtprotoService.isClientConnected()) {
    return [];
  }

  const client = mtprotoService.getClient();

  // Calculate addOffset based on direction
  let addOffset: number;
  switch (direction) {
    case "around":
      addOffset = -Math.round(slice / 2) - 1;
      break;
    case "backward":
      addOffset = -1;
      break;
    case "forward":
      addOffset = -(slice + 1);
      break;
  }

  try {
    // Check abort before API call
    if (controller.signal.aborted) return [];

    const apiMessages = await mtprotoService.withFloodWaitHandling(async () => {
      if (threadId && threadId !== MAIN_THREAD_ID) {
        // Thread replies
        return client.invoke(
          new Api.messages.GetReplies({
            peer: chatId,
            msgId: Number(threadId),
            offsetId: offsetId ?? 0,
            addOffset,
            limit: slice,
          }),
        );
      }
      // Regular chat history
      return client.invoke(
        new Api.messages.GetHistory({
          peer: chatId,
          offsetId: offsetId ?? 0,
          addOffset,
          limit: slice,
        }),
      );
    });

    // Check abort after API call
    if (controller.signal.aborted) return [];

    // Convert API messages to our format
    const messages = convertApiMessages(apiMessages, chatId);

    if (messages.length === 0) return [];

    // Dispatch to Redux
    const userId = selectTelegramCurrentUserId(store.getState());
    if (!userId) return messages;

    store.dispatch(
      addChatMessagesById({
        userId,
        chatId,
        messages,
      }),
    );

    // Update viewport IDs (only for primary loads, not budget preloads)
    if (!isBudgetPreload) {
      const viewportLimit = getViewportLimit();
      const messageIds = messages.map((m) => m.id);
      const viewportIds =
        messageIds.length > viewportLimit
          ? messageIds.slice(-viewportLimit)
          : messageIds;

      store.dispatch(
        setViewportIds({
          userId,
          chatId,
          threadId,
          viewportIds,
        }),
      );
    }

    // For 'around' direction with a jump target, track as outlying if it doesn't
    // connect to existing listed IDs
    if (direction === "around" && offsetId) {
      const state = store.getState();
      const existing =
        state.telegram.byUser[userId]?.messagesOrder[chatId] ?? [];
      const newIds = messages.map((m) => m.id);

      // Check if there's a gap between existing messages and new ones
      if (existing.length > 0 && newIds.length > 0) {
        const existingSet = new Set(existing);
        const hasOverlap = newIds.some((id) => existingSet.has(id));
        if (!hasOverlap) {
          store.dispatch(
            addOutlyingList({
              userId,
              chatId,
              threadId,
              ids: newIds,
            }),
          );
        }
      }
    }

    return messages;
  } catch (error) {
    if (controller.signal.aborted) return [];
    console.error(`[MessageLoader] Failed to load messages for ${chatId}:`, error);
    throw error;
  }
}

// ---------------------------------------------------------------------------
// Budget preloading (two-pass)
// ---------------------------------------------------------------------------

/**
 * Two-pass message loading:
 * 1. First pass fills the viewport
 * 2. Second pass fetches additional buffer beyond the viewport
 */
export async function loadMessagesWithBudget(
  chatId: string,
  offsetId?: number,
  threadId?: string,
): Promise<TelegramMessage[]> {
  // Pass 1: Fill viewport
  const viewportMessages = await loadMessages({
    chatId,
    direction: offsetId ? "around" : "backward",
    offsetId,
    threadId,
  });

  if (viewportMessages.length === 0) return [];

  // Pass 2: Budget preload — fetch buffer beyond viewport
  const slice = getSliceSize();
  if (viewportMessages.length >= slice) {
    // There may be more messages — preload buffer
    const oldestId = Number(viewportMessages[0].id);
    try {
      await loadMessages({
        chatId,
        direction: "backward",
        offsetId: oldestId,
        threadId,
        isBudgetPreload: true,
      });
    } catch {
      // Budget preload failure is non-critical
    }
  }

  return viewportMessages;
}

// ---------------------------------------------------------------------------
// Batch ID fetching
// ---------------------------------------------------------------------------

/**
 * Fetch specific messages by their IDs (up to 100 per request).
 * Uses messages.GetMessages for regular chats or channels.GetMessages for channels.
 */
export async function fetchMessagesByIds(
  chatId: string,
  messageIds: number[],
  isChannel = false,
): Promise<TelegramMessage[]> {
  if (!mtprotoService.isReady() || !mtprotoService.isClientConnected()) {
    return [];
  }

  const client = mtprotoService.getClient();
  const allMessages: TelegramMessage[] = [];

  // Process in batches of BATCH_FETCH_LIMIT
  for (let i = 0; i < messageIds.length; i += BATCH_FETCH_LIMIT) {
    const batch = messageIds.slice(i, i + BATCH_FETCH_LIMIT);
    const inputIds = batch.map(
      (id) => new Api.InputMessageID({ id }),
    );

    try {
      const result = await mtprotoService.withFloodWaitHandling(async () => {
        if (isChannel) {
          return client.invoke(
            new Api.channels.GetMessages({
              channel: chatId,
              id: inputIds,
            }),
          );
        }
        return client.invoke(
          new Api.messages.GetMessages({
            id: inputIds,
          }),
        );
      });

      const messages = convertApiMessages(result, chatId);
      allMessages.push(...messages);
    } catch (error) {
      console.error(
        `[MessageLoader] Batch fetch failed for ${chatId} (batch ${i / BATCH_FETCH_LIMIT}):`,
        error,
      );
    }
  }

  // Dispatch to Redux
  if (allMessages.length > 0) {
    const userId = selectTelegramCurrentUserId(store.getState());
    if (userId) {
      store.dispatch(
        addChatMessagesById({
          userId,
          chatId,
          messages: allMessages,
        }),
      );
    }
  }

  return allMessages;
}

// ---------------------------------------------------------------------------
// API response conversion
// ---------------------------------------------------------------------------

/**
 * Convert Telegram API message objects to our TelegramMessage format.
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function convertApiMessages(result: any, chatId: string): TelegramMessage[] {
  const messages: TelegramMessage[] = [];

  const rawMessages: unknown[] =
    result?.messages ?? (Array.isArray(result) ? result : []);

  for (const raw of rawMessages) {
    const msg = raw as Record<string, unknown>;
    if (!msg || typeof msg !== "object") continue;

    // Skip empty/service messages without content
    const id = msg.id;
    if (id === undefined || id === null) continue;

    const telegramMsg: TelegramMessage = {
      id: String(id),
      chatId,
      date: typeof msg.date === "number" ? msg.date : 0,
      message: typeof msg.message === "string" ? msg.message : "",
      isOutgoing: Boolean(msg.out),
      isEdited: Boolean(msg.editDate),
      isForwarded: Boolean(msg.fwdFrom),
    };

    // From ID
    if (msg.fromId && typeof msg.fromId === "object") {
      const fromId = msg.fromId as Record<string, unknown>;
      telegramMsg.fromId = String(
        fromId.userId ?? fromId.channelId ?? fromId.chatId ?? "",
      );
    }

    // Reply
    if (msg.replyTo && typeof msg.replyTo === "object") {
      const replyTo = msg.replyTo as Record<string, unknown>;
      if (replyTo.replyToMsgId) {
        telegramMsg.replyToMessageId = String(replyTo.replyToMsgId);
      }
    }

    // Thread ID
    if (msg.replyTo && typeof msg.replyTo === "object") {
      const replyTo = msg.replyTo as Record<string, unknown>;
      if (replyTo.replyToTopId) {
        telegramMsg.threadId = String(replyTo.replyToTopId);
      }
    }

    // Media
    if (msg.media && typeof msg.media === "object") {
      const media = msg.media as Record<string, unknown>;
      const className = (media.constructor as { className?: string })?.className;
      telegramMsg.media = {
        type: className ?? "unknown",
      };
    }

    // Views
    if (typeof msg.views === "number") {
      telegramMsg.views = msg.views;
    }

    messages.push(telegramMsg);
  }

  return messages;
}
