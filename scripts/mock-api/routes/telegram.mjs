/**
 * Mock Telegram Bot API route handler.
 *
 * Intercepts all requests matching `/bot<token>/<method>` and responds with
 * realistic Telegram Bot API shapes. Behaviour is driven by the mock behavior
 * keys documented below so E2E specs can configure failure modes without
 * changing code.
 *
 * Behavior keys:
 *   telegramBotUsername   - bot username returned by getMe (default: "e2e_test_bot")
 *   telegramBotToken      - expected bot token; if set, mismatches return 401
 *   telegramPollDelayMs   - delay before getUpdates returns (simulates long-poll)
 *   telegramGetMeFails    - if "1", getMe returns HTTP 401 Unauthorized
 *   telegramSendFails     - if "1", sendMessage returns HTTP 400 Bad Request
 */

import { json } from "../http.mjs";
import {
  behavior,
  drainMockTelegramUpdates,
  nextMockTelegramMessageId,
  recordMockTelegramSent,
  sleep,
} from "../state.mjs";

/** Extract the bot token and method name from a Telegram Bot API path. */
const BOT_PATH_RE = /^\/bot([^/]+)\/([^/?]+)/;

/**
 * Attempt to parse a Telegram Bot API path.
 * Returns `{ token, method }` or `null` if the path does not match.
 */
function parseBotPath(url) {
  const m = BOT_PATH_RE.exec(url);
  if (!m) return null;
  return { token: m[1], method: m[2] };
}

/**
 * Main route handler. Returns `true` when the request was handled, `false`
 * when it should fall through to the next handler.
 */
export async function handleTelegram(ctx) {
  const { url, parsedBody, res } = ctx;

  const parsed = parseBotPath(url);
  if (!parsed) return false;

  const { token, method } = parsed;
  const b = behavior();

  console.log(`[telegram-mock] ${method} token=${token.slice(0, 8)}...`);

  // Optional token validation — only enforced when behavior key is set.
  if (b.telegramBotToken && token !== b.telegramBotToken) {
    console.warn(
      `[telegram-mock] token mismatch: got ${token.slice(0, 8)}... expected ${b.telegramBotToken.slice(0, 8)}...`,
    );
    json(res, 401, {
      ok: false,
      error_code: 401,
      description: "Unauthorized",
    });
    return true;
  }

  switch (method) {
    case "getMe":
      return handleGetMe(res, b);

    case "getUpdates":
      return handleGetUpdates(res, b);

    case "sendMessage":
      return handleSendMessage(res, b, parsedBody);

    case "sendChatAction":
      return handleSimpleRecord(res, "sendChatAction", parsedBody);

    case "deleteWebhook":
      return handleSimpleRecord(res, "deleteWebhook", parsedBody);

    case "setMessageReaction":
      return handleSimpleRecord(res, "setMessageReaction", parsedBody);

    case "editMessageText":
      return handleSimpleRecord(res, "editMessageText", parsedBody);

    case "editMessageReplyMarkup":
      return handleSimpleRecord(res, "editMessageReplyMarkup", parsedBody);

    case "answerCallbackQuery":
      return handleSimpleRecord(res, "answerCallbackQuery", parsedBody);

    case "sendPhoto":
    case "sendDocument":
    case "sendVideo":
    case "sendAudio":
    case "sendVoice":
    case "sendAnimation":
    case "sendSticker": {
      // Multipart bodies are not parsed — just record the method and any JSON
      // keys we received (body may be null for multipart).
      const messageId = nextMockTelegramMessageId();
      recordMockTelegramSent({
        method,
        body: parsedBody ?? {},
        message_id: messageId,
      });
      console.log(
        `[telegram-mock] ${method} recorded message_id=${messageId}`,
      );
      json(res, 200, { ok: true, result: { message_id: messageId } });
      return true;
    }

    default:
      console.log(
        `[telegram-mock] unhandled method="${method}" — returning ok:true, result:null`,
      );
      json(res, 200, { ok: true, result: null });
      return true;
  }
}

// ── Individual method handlers ─────────────────────────────────────────────

function handleGetMe(res, b) {
  if (b.telegramGetMeFails === "1") {
    console.warn("[telegram-mock] getMe failing per behavior.telegramGetMeFails");
    json(res, 401, {
      ok: false,
      error_code: 401,
      description: "Unauthorized",
    });
    return true;
  }

  const username = b.telegramBotUsername || "e2e_test_bot";
  console.log(`[telegram-mock] getMe -> username=${username}`);
  json(res, 200, {
    ok: true,
    result: {
      id: 123456789,
      is_bot: true,
      first_name: "E2E Bot",
      username,
    },
  });
  return true;
}

async function handleGetUpdates(res, b) {
  const delayMs = Math.min(
    Number(b.telegramPollDelayMs) || 50,
    30_000,
  );
  if (Number.isFinite(delayMs) && delayMs > 0) {
    console.log(`[telegram-mock] getUpdates: waiting ${delayMs}ms before reply`);
    await sleep(delayMs);
  }

  const updates = drainMockTelegramUpdates();
  console.log(`[telegram-mock] getUpdates: returning ${updates.length} update(s)`);
  json(res, 200, { ok: true, result: updates });
  return true;
}

function handleSendMessage(res, b, body) {
  if (b.telegramSendFails === "1") {
    console.warn("[telegram-mock] sendMessage failing per behavior.telegramSendFails");
    json(res, 400, {
      ok: false,
      error_code: 400,
      description: "Bad Request",
    });
    return true;
  }

  const messageId = nextMockTelegramMessageId();
  const chatId = body?.chat_id ?? 0;
  const text = body?.text ?? "";

  recordMockTelegramSent({
    method: "sendMessage",
    body: body ?? {},
    message_id: messageId,
  });

  console.log(
    `[telegram-mock] sendMessage chat_id=${chatId} message_id=${messageId} text="${String(text).slice(0, 80)}"`,
  );

  json(res, 200, {
    ok: true,
    result: {
      message_id: messageId,
      date: Math.floor(Date.now() / 1000),
      chat: { id: chatId },
      text,
    },
  });
  return true;
}

function handleSimpleRecord(res, method, body) {
  recordMockTelegramSent({ method, body: body ?? {} });
  console.log(`[telegram-mock] ${method} recorded`);
  json(res, 200, { ok: true, result: true });
  return true;
}
