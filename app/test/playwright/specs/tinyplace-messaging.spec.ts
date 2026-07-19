// UI e2e: tiny.place direct messaging through the real openhuman Messaging
// screen, against a real tiny.place backend.
//
// The app-under-test (Alice) talks to the standalone core the web session
// booted (PW_CORE_RPC_URL). A second real openhuman-core (Bob) is launched here
// as the peer. Both point at the same tiny.place backend
// (TINYPLACE_API_BASE_URL). We establish an accepted contact out-of-band (the
// contact request/accept flow itself is covered exhaustively by the core suite
// at e2e/tinyplace-messaging/messaging.e2e.mjs), then drive the DM UI:
//
//   • type a recipient + open the DM thread
//   • send an end-to-end encrypted message from the UI → assert the real peer
//     core receives and decrypts it
//   • have the peer reply → assert the plaintext renders in the UI thread
//
// Requires the web session harness (app/scripts/e2e-web-session.sh) with
// TINYPLACE_API_BASE_URL exported so the core hits a real backend. The
// messaging e2e runner (e2e/tinyplace-messaging/run-ui.sh) wires this up.
import { expect, test } from '@playwright/test';

// The core-launch helper is shared with the core-level suite (plain ESM).
import { launchAgent, receiveMessage } from '../../../../e2e/tinyplace-messaging/lib/core.mjs';
import { bootAuthenticatedPage } from '../helpers/core-rpc';

const CORE_RPC_URL = process.env.PW_CORE_RPC_URL || 'http://127.0.0.1:17788/rpc';
const CORE_RPC_TOKEN = process.env.PW_CORE_RPC_TOKEN || 'openhuman-playwright-token';
const BACKEND = process.env.TINYPLACE_API_BASE_URL || 'http://localhost:18080';
const HAS_TINYPLACE_BACKEND = Boolean(process.env.TINYPLACE_API_BASE_URL);

const TEST_MNEMONIC_WORDS = 12;
// A fresh, valid BIP-39 mnemonic for Alice (the app's core identity). Generated
// via the same dependency-free generator the core suite uses.
async function freshMnemonic(): Promise<string> {
  const { generateMnemonic } = await import('../../../../e2e/tinyplace-messaging/lib/mnemonic.mjs');
  const m = generateMnemonic();
  if (m.split(' ').length !== TEST_MNEMONIC_WORDS) throw new Error('unexpected mnemonic length');
  return m;
}

const PLACEHOLDER_ACCOUNTS = [
  {
    chain: 'evm',
    address: '0x0000000000000000000000000000000000000001',
    derivationPath: "m/44'/60'/0'/0/0",
  },
  {
    chain: 'btc',
    address: 'bc1qplaceholderplaceholderplaceholderplac0000',
    derivationPath: "m/84'/0'/0'/0/0",
  },
  {
    chain: 'solana',
    address: '11111111111111111111111111111111',
    derivationPath: "m/44'/501'/0'/0'",
  },
  {
    chain: 'tron',
    address: 'T0000000000000000000000000000000001',
    derivationPath: "m/44'/195'/0'/0/0",
  },
];

/** Call Alice's (the app's) core over JSON-RPC and unwrap the {logs,result}. */
async function aliceRpc<T = any>(method: string, params: Record<string, unknown> = {}): Promise<T> {
  const res = await fetch(CORE_RPC_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${CORE_RPC_TOKEN}` },
    body: JSON.stringify({ jsonrpc: '2.0', id: Date.now(), method, params }),
  });
  const body = await res.json();
  if (body.error) throw new Error(`${method} -> ${JSON.stringify(body.error)}`);
  let result = body.result;
  if (result && typeof result === 'object' && 'result' in result && 'logs' in result) {
    result = result.result;
  }
  return result as T;
}

let bob: Awaited<ReturnType<typeof launchAgent>>;
let aliceCryptoId: string;

test.describe('tiny.place direct messaging (UI)', () => {
  test.skip(
    !HAS_TINYPLACE_BACKEND,
    'requires TINYPLACE_API_BASE_URL from the dedicated tiny.place E2E runner'
  );
  test.describe.configure({ mode: 'serial' });

  test.beforeAll(async () => {
    // 1) Give the app's core a fresh tiny.place identity + published Signal keys.
    const mnemonic = await freshMnemonic();
    const encryptedMnemonic = await aliceRpc<string>('openhuman.encrypt_secret', {
      plaintext: mnemonic,
    });
    await aliceRpc('openhuman.wallet_setup', {
      consentGranted: true,
      source: 'imported',
      mnemonicWordCount: TEST_MNEMONIC_WORDS,
      encryptedMnemonic,
      accounts: PLACEHOLDER_ACCOUNTS,
      force: true,
    });
    await aliceRpc('openhuman.tinyplace_signal_provision', { preKeyCount: 10 });
    await aliceRpc('openhuman.tinyplace_signal_register_encryption_key', {});
    const status = await aliceRpc<{ agentId: string }>('openhuman.tinyplace_signal_key_status', {});
    aliceCryptoId = status.agentId;
    expect(aliceCryptoId, 'app core produced a cryptoId').toBeTruthy();

    // 2) Launch the peer core (Bob) and make Alice + Bob accepted contacts so
    //    the relay will carry their DMs.
    bob = await launchAgent('pw-bob', { port: 17851, backend: BACKEND });
    await aliceRpc('openhuman.tinyplace_contacts_request', { agentId: bob.cryptoId });
    await bob.rpc('openhuman.tinyplace_contacts_accept', { agentId: aliceCryptoId });
  });

  test.afterAll(() => {
    bob?.stop();
  });

  test('sends an encrypted DM from the UI that the peer decrypts, and renders the peer reply', async ({
    page,
  }) => {
    await bootAuthenticatedPage(page, 'pw-messaging-user', '/agent-world/messaging');

    // The DM composer: enter the peer's cryptoId and open the thread.
    const recipient = page.getByPlaceholder('Recipient @handle or wallet address');
    await expect(recipient).toBeVisible();
    await recipient.fill(bob.cryptoId);
    await page.getByRole('button', { name: 'Open DM' }).click();

    // We're in the encrypted thread with Bob (the header lock badge + empty state).
    await expect(page.getByText('Encrypted', { exact: true })).toBeVisible();
    await expect(page.getByTestId('dm-empty-state')).toBeVisible();

    // Send an end-to-end encrypted message from the UI.
    const outgoing = `ui → peer @ ${Date.now()}`;
    const compose = page.getByPlaceholder('Type a message...');
    await compose.fill(outgoing);
    await page.getByRole('button', { name: 'Send' }).click();

    // Optimistic echo appears in the thread.
    await expect(page.getByText(outgoing)).toBeVisible();

    // The real peer core receives + decrypts exactly what the UI sent.
    const received = await receiveMessage(bob, { fromCryptoId: aliceCryptoId, timeoutMs: 12_000 });
    expect(received, 'peer decrypts the message sent from the UI').toBe(outgoing);

    // Now the peer replies; the plaintext must render in the UI thread.
    const reply = `peer → ui @ ${Date.now()}`;
    await bob.rpc('openhuman.tinyplace_signal_send_message', {
      recipient: aliceCryptoId,
      plaintext: reply,
    });

    // Wait until the reply envelope is actually in Alice's mailbox — inspected,
    // not decrypted (decrypt advances the ratchet and can only run once, so we
    // must not consume it before the UI does).
    await expect
      .poll(
        async () => {
          const list = await aliceRpc<any>('openhuman.tinyplace_messages_list', { limit: 50 });
          const envelopes = Array.isArray(list) ? list : (list?.messages ?? []);
          return envelopes.some((e: any) => e.from === bob.cryptoId);
        },
        { timeout: 15_000, intervals: [500, 1000] }
      )
      .toBe(true);

    // Re-open the thread once. The DM view has no interval poll; its mount fetch
    // is what decrypts + renders the delivered reply.
    await page.getByRole('button', { name: 'Back', exact: true }).click();
    await page.getByPlaceholder('Recipient @handle or wallet address').fill(bob.cryptoId);
    await page.getByRole('button', { name: 'Open DM' }).click();
    await expect(page.getByText(reply)).toBeVisible({ timeout: 15_000 });
  });
});
