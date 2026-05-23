/**
 * Telegram channel E2E helpers.
 *
 * Wraps `callOpenhumanRpc` (core RPC) and the Telegram mock admin endpoints so
 * specs can drive the full Telegram channel lifecycle without knowing the raw
 * RPC method names or admin path strings.
 *
 * Design principles:
 *  - All helpers are pure async functions — no hidden state.
 *  - Admin HTTP helpers call mock server endpoints that are already wired by
 *    WS-A (see `app/test/e2e/mock-server.ts` and `scripts/mock-api/routes/telegram.mjs`).
 *  - RPC helpers forward to `callOpenhumanRpc` using the exact field names from
 *    `src/openhuman/channels/controllers/schemas.rs` (camelCase for the wire
 *    format; the Rust serde layer translates).
 *
 * Key RPC shapes (verified from schemas.rs / ops.rs):
 *   channels_connect  -> { channel, authMode, credentials: { bot_token, allowed_users?, mention_only? } }
 *   channels_disconnect -> { channel, authMode }
 *   channels_status   -> { channel? }   -> entries: ChannelStatusEntry[]
 */
import {
  getTelegramSentMessages as adminGetSentMessages,
  injectTelegramUpdate as adminInjectUpdate,
  resetTelegramMock as adminReset,
} from '../mock-server';
import { callOpenhumanRpc } from './core-rpc';

const LOG_PREFIX = '[TelegramChannel]';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface TelegramConnectOptions {
  /** Bot token issued by BotFather. */
  botToken: string;
  /**
   * Optional allowlist of Telegram usernames (without `@`) or numeric user
   * IDs. When provided only those identities can trigger the bot. When
   * omitted the bot uses the pairing-code flow.
   */
  allowedUsers?: string[];
  /**
   * When true the bot only responds to messages that mention it by
   * `@username` in group chats.
   */
  mentionOnly?: boolean;
}

export interface TelegramConnectResult {
  ok: boolean;
  status?: string;
  restartRequired?: boolean;
  message?: string;
  error?: string;
}

export interface TelegramStatusEntry {
  channelId: string;
  authMode: string;
  connected: boolean;
  hasCredentials: boolean;
}

export interface TelegramUpdate {
  update_id: number;
  message: {
    message_id: number;
    from: { id: number; is_bot: boolean; first_name: string; username?: string };
    chat: {
      id: number;
      type: 'private' | 'group' | 'supergroup' | 'channel';
      title?: string;
      username?: string;
      first_name?: string;
    };
    date: number;
    text: string;
  };
}

export interface SentMessage {
  method: string;
  chat_id: string | number;
  text?: string;
  [key: string]: unknown;
}

// ---------------------------------------------------------------------------
// RPC wrappers
// ---------------------------------------------------------------------------

/**
 * Connect a Telegram bot via the `channels_connect` RPC.
 *
 * Maps to `openhuman.channels_connect` with `authMode: "bot_token"`.
 * The connect call writes TOML config and sets `restart_required: true` —
 * it does NOT start the live polling loop immediately.
 *
 * Returns the raw RPC result so callers can assert on specific fields.
 */
export async function connectTelegramBot(
  opts: TelegramConnectOptions
): Promise<TelegramConnectResult> {
  const credentials: Record<string, unknown> = { bot_token: opts.botToken };
  if (opts.allowedUsers !== undefined) {
    credentials.allowed_users = opts.allowedUsers;
  }
  if (opts.mentionOnly !== undefined) {
    credentials.mention_only = opts.mentionOnly;
  }

  console.log(
    `${LOG_PREFIX} connectTelegramBot: token=***${opts.botToken.slice(-4)} ` +
      `allowedUsers=${JSON.stringify(opts.allowedUsers ?? [])} ` +
      `mentionOnly=${opts.mentionOnly ?? false}`
  );

  const out = await callOpenhumanRpc('openhuman.channels_connect', {
    channel: 'telegram',
    authMode: 'bot_token',
    credentials,
  });

  if (!out.ok) {
    console.warn(`${LOG_PREFIX} connectTelegramBot: RPC failed — ${JSON.stringify(out)}`);
    return { ok: false, error: String(out.error ?? 'unknown error') };
  }

  // The result shape from ops.rs is { status, restart_required, message? }.
  // It is wrapped by RpcOutcome which the Node RPC client unwraps one level.
  const result = (out.result as Record<string, unknown> | null) ?? {};
  const inner =
    typeof result.result === 'object' && result.result !== null
      ? (result.result as Record<string, unknown>)
      : result;

  console.log(`${LOG_PREFIX} connectTelegramBot: ok — ${JSON.stringify(inner)}`);
  return {
    ok: true,
    status: inner.status as string | undefined,
    restartRequired: inner.restart_required as boolean | undefined,
    message: inner.message as string | undefined,
  };
}

/**
 * Disconnect the Telegram bot via `channels_disconnect` RPC.
 *
 * Removes stored credentials and clears TOML config. Returns true on success.
 */
export async function disconnectTelegramBot(): Promise<boolean> {
  console.log(`${LOG_PREFIX} disconnectTelegramBot: calling channels_disconnect`);

  const out = await callOpenhumanRpc('openhuman.channels_disconnect', {
    channel: 'telegram',
    authMode: 'bot_token',
  });

  if (!out.ok) {
    console.warn(`${LOG_PREFIX} disconnectTelegramBot: RPC failed — ${JSON.stringify(out)}`);
    return false;
  }

  console.log(`${LOG_PREFIX} disconnectTelegramBot: ok`);
  return true;
}

/**
 * Fetch the channel status for Telegram.
 *
 * Calls `channels_status` with `channel: "telegram"` and returns the first
 * matching entry (the `bot_token` mode entry). Returns `null` if no entry is
 * found or the RPC fails.
 */
export async function getTelegramChannelStatus(): Promise<TelegramStatusEntry | null> {
  console.log(`${LOG_PREFIX} getTelegramChannelStatus: calling channels_status`);

  const out = await callOpenhumanRpc('openhuman.channels_status', { channel: 'telegram' });

  if (!out.ok) {
    console.warn(`${LOG_PREFIX} getTelegramChannelStatus: RPC failed — ${JSON.stringify(out)}`);
    return null;
  }

  // channels_status returns entries: ChannelStatusEntry[].
  // The core wraps with RpcOutcome so the Node client may unwrap one level.
  const result = (out.result as Record<string, unknown> | null) ?? {};
  const entries: TelegramStatusEntry[] = Array.isArray(result)
    ? result
    : Array.isArray((result as Record<string, unknown>).entries)
      ? ((result as Record<string, unknown>).entries as TelegramStatusEntry[])
      : Array.isArray((result as Record<string, unknown>).result)
        ? ((result as Record<string, unknown>).result as TelegramStatusEntry[])
        : [];

  const match = entries.find(
    (e: TelegramStatusEntry) =>
      (e.channelId === 'telegram' || e.channel_id === 'telegram') &&
      (e.authMode === 'bot_token' || (e as Record<string, unknown>).auth_mode === 'bot_token')
  ) as TelegramStatusEntry | undefined;

  console.log(`${LOG_PREFIX} getTelegramChannelStatus: ${JSON.stringify(match ?? null)}`);
  return match ?? null;
}

// ---------------------------------------------------------------------------
// Mock admin helpers (relay to WS-A admin endpoints)
// ---------------------------------------------------------------------------

/**
 * Build a realistic Telegram Update JSON for a private or group message.
 *
 * @param opts.updateId    — Telegram update_id (must increase monotonically).
 * @param opts.chatId      — Chat numeric ID.
 * @param opts.userId      — Sender numeric user ID.
 * @param opts.username    — Sender Telegram username (without `@`).
 * @param opts.text        — Message text.
 * @param opts.isGroup     — When true, emits a group chat type.
 * @param opts.botUsername — When provided AND isGroup, includes the mention
 *                           in the text so mention-only filtering fires.
 */
export function buildTelegramUpdate(opts: {
  updateId: number;
  chatId: number;
  userId: number;
  username: string;
  text: string;
  isGroup?: boolean;
  botUsername?: string;
}): TelegramUpdate {
  const chatType = opts.isGroup ? 'group' : 'private';

  return {
    update_id: opts.updateId,
    message: {
      message_id: opts.updateId * 10, // stable across retries
      from: { id: opts.userId, is_bot: false, first_name: opts.username, username: opts.username },
      chat: {
        id: opts.chatId,
        type: chatType,
        ...(opts.isGroup ? { title: `e2e-group-${opts.chatId}` } : { first_name: opts.username }),
      },
      date: Math.floor(Date.now() / 1000),
      text: opts.text,
    },
  };
}

/**
 * Inject a Telegram Update into the mock server's pending queue.
 * The Telegram provider's `getUpdates` poll will drain this on the next call.
 */
export async function injectTelegramUpdate(update: TelegramUpdate): Promise<void> {
  console.log(
    `${LOG_PREFIX} injectTelegramUpdate: update_id=${update.update_id} ` +
      `chat_id=${update.message.chat.id} text="${update.message.text.slice(0, 60)}"`
  );
  await adminInjectUpdate(update);
}

/**
 * Poll the mock's sent-messages log until a `sendMessage` entry appears that
 * matches the given `chatId` and optional `contains` predicate.
 *
 * Returns the matching entry, or throws after `timeoutMs`.
 */
export async function waitForTelegramReply(opts: {
  chatId: number;
  contains?: string;
  timeoutMs?: number;
}): Promise<SentMessage> {
  const { chatId, contains, timeoutMs = 20_000 } = opts;

  console.log(
    `${LOG_PREFIX} waitForTelegramReply: chatId=${chatId} ` +
      `contains="${contains ?? '*'}" timeout=${timeoutMs}ms`
  );

  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    const sent = (await adminGetSentMessages()) as SentMessage[];
    const match = sent.find(entry => {
      // method might be 'sendMessage', 'sendText', etc.; filter by chat_id first.
      const matchesChat =
        String(entry.chat_id) === String(chatId) ||
        // Some mock implementations nest the chat_id inside a request body JSON.
        String((entry as Record<string, unknown>).body_chat_id ?? '') === String(chatId);

      if (!matchesChat) return false;
      if (!contains) return true;

      const text = String(entry.text ?? entry.message ?? '');
      return text.includes(contains);
    });

    if (match) {
      console.log(`${LOG_PREFIX} waitForTelegramReply: found match — ${JSON.stringify(match)}`);
      return match;
    }

    await browser.pause(300);
  }

  const allSent = (await adminGetSentMessages()) as SentMessage[];
  throw new Error(
    `${LOG_PREFIX} waitForTelegramReply: TIMEOUT — no reply to chatId=${chatId}` +
      (contains ? ` containing "${contains}"` : '') +
      ` after ${timeoutMs}ms. Sent log (${allSent.length} entries): ` +
      JSON.stringify(allSent.slice(-5))
  );
}

/**
 * Poll the mock's sent-messages log and assert that NO reply to `chatId`
 * appears within `timeoutMs`. Returns `true` if the window passes cleanly,
 * `false` if a matching message is observed.
 */
export async function assertNoTelegramReply(opts: {
  chatId: number;
  contains?: string;
  timeoutMs?: number;
}): Promise<boolean> {
  const { chatId, contains, timeoutMs = 5_000 } = opts;

  console.log(
    `${LOG_PREFIX} assertNoTelegramReply: chatId=${chatId} ` +
      `contains="${contains ?? '*'}" window=${timeoutMs}ms`
  );

  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    const sent = (await adminGetSentMessages()) as SentMessage[];
    const match = sent.find(entry => {
      const matchesChat = String(entry.chat_id) === String(chatId);
      if (!matchesChat) return false;
      if (!contains) return true;
      const text = String(entry.text ?? entry.message ?? '');
      return text.includes(contains);
    });

    if (match) {
      console.warn(
        `${LOG_PREFIX} assertNoTelegramReply: UNEXPECTED reply to chatId=${chatId} — ` +
          JSON.stringify(match)
      );
      return false;
    }

    await browser.pause(300);
  }

  console.log(`${LOG_PREFIX} assertNoTelegramReply: clean — no reply in ${timeoutMs}ms window`);
  return true;
}

/**
 * Reset the Telegram mock state (pending update queue + sent log + counter).
 * Delegates to the WS-A admin endpoint via `mock-server.ts`.
 */
export async function resetTelegramMock(): Promise<void> {
  await adminReset();
  console.log(`${LOG_PREFIX} resetTelegramMock: done`);
}
