// @ts-nocheck
/**
 * E2E mock server wrapper.
 *
 * Re-exports the shared mock backend used by app unit tests, app E2E,
 * and Rust tests (via scripts/mock-api-server.mjs + scripts/test-rust-with-mock.sh).
 */
export {
  clearRequestLog,
  emitMockAgentAudioStream,
  getMockBehavior,
  getMockServerPort,
  getRequestLog,
  resetMockBehavior,
  setMockBehavior,
  setMockBehaviors,
  startMockServer,
  stopMockServer,
} from '../../../scripts/mock-api-core.mjs';

// ── Telegram mock helpers ──────────────────────────────────────────────────
// Convenience wrappers for E2E specs that drive the Telegram channel.
// These call the admin HTTP endpoints so they work from the WDIO process
// (which cannot import the mock server module directly when it is running
// in a separate process).

async function telegramAdminFetch(path, options) {
  // Resolve port lazily so this module can be imported before the server
  // is started. The `getMockServerPort` export above resolves at call time.
  const { getMockServerPort } = await import('../../../scripts/mock-api/index.mjs');
  const port = getMockServerPort();
  if (!port) throw new Error('[mock-server] mock server is not running');
  return fetch(`http://127.0.0.1:${port}${path}`, options);
}

/**
 * Inject one Telegram Update into the mock server's pending queue.
 * The Telegram provider will receive it on the next `getUpdates` poll.
 *
 * @param update - A Telegram Update object (https://core.telegram.org/bots/api#update)
 */
export async function injectTelegramUpdate(update) {
  const res = await telegramAdminFetch('/__admin/telegram/inject-update', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(update),
  });
  return res.json();
}

/**
 * Return all outbound Telegram API calls that the bot has made since the
 * last reset. Useful for asserting the bot's reply text and method.
 */
export async function getTelegramSentMessages() {
  const res = await telegramAdminFetch('/__admin/telegram/sent');
  return res.json();
}

/**
 * Clear the Telegram mock state (pending update queue + sent messages log +
 * message_id counter). Does NOT affect other mock state.
 */
export async function resetTelegramMock() {
  const res = await telegramAdminFetch('/__admin/telegram/reset', { method: 'POST' });
  return res.json();
}
