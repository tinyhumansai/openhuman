/**
 * Chat harness + wallet flow end-to-end.
 *
 * This spec locks down the current wallet contract as it exists today:
 *
 *   1. A user can complete local wallet setup through the real
 *      Recovery Phrase panel, which persists `state/wallet-state.json`
 *      in the workspace.
 *   2. A real `/chat` turn can route through the orchestrator into the
 *      crypto sub-agent and invoke `wallet_prepare_transfer`.
 *   3. The resulting prepared quote is visible from Rust-side
 *      test-support introspection, proving the agent flow crossed the
 *      UI → core → tool-dispatch boundary.
 *
 * What this does NOT claim:
 *   - on-chain broadcast
 *   - desktop-keystore signing
 *
 * The current core implementation explicitly stops at
 * `execute_prepared -> ReadyToSign`, so the E2E keeps its assertion
 * surface honest and checks the prepared quote boundary.
 */
import { waitForApp } from '../helpers/app-helpers';
import {
  clickByTitle,
  clickSend,
  getSelectedThreadId,
  hexEncodeThreadId,
  typeIntoComposer,
} from '../helpers/chat-harness';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { clickText, clickToggle, textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import {
  clearRequestLog,
  getRequestLog,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

const USER_ID = 'e2e-chat-harness-wallet-flow';
const CANARY = 'wallet-quote-canary-8d13';
const JOHN_ADDRESS = '0x00000000000000000000000000000000000000aa';
const WALLET_PROMPT = `Send John $5 on EVM at ${JOHN_ADDRESS} and tell me ${CANARY}.`;

const FORCED_RESPONSES = [
  {
    content: '',
    toolCalls: [
      {
        id: 'call_delegate_do_crypto_1',
        name: 'delegate_do_crypto',
        arguments: JSON.stringify({
          prompt: `Prepare a $5 EVM transfer to John at ${JOHN_ADDRESS}.`,
        }),
      },
    ],
  },
  {
    content: '',
    toolCalls: [{ id: 'call_wallet_status_1', name: 'wallet_status', arguments: '{}' }],
  },
  {
    content: '',
    toolCalls: [{ id: 'call_wallet_chain_status_1', name: 'wallet_chain_status', arguments: '{}' }],
  },
  {
    content: '',
    toolCalls: [
      {
        id: 'call_wallet_prepare_transfer_1',
        name: 'wallet_prepare_transfer',
        arguments: JSON.stringify({
          chain: 'evm',
          toAddress: JOHN_ADDRESS,
          amountRaw: '5000000000000000000',
        }),
      },
    ],
  },
  { content: `Prepared a wallet quote for John. ${CANARY}` },
  { content: `Done. ${CANARY}` },
];

async function clickRecoveryConsentCheckbox(): Promise<void> {
  const checkbox = await browser.$('input[type="checkbox"]');
  if (!(await checkbox.isExisting())) {
    throw new Error('Recovery phrase consent checkbox not found');
  }
  if (!(await checkbox.isSelected())) {
    await clickToggle();
  }
  await expect(checkbox).toBeSelected();
}

describe('Chat harness — wallet flow', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    setMockBehavior('llmForcedResponses', '');
    setMockBehavior('llmStreamChunkDelayMs', '');
    await stopMockServer();
  });

  it('sets up the local wallet through the Recovery Phrase panel and persists wallet state', async () => {
    await navigateViaHash('/settings/recovery-phrase');
    await browser.waitUntil(async () => await textExists('Save Recovery Phrase'), {
      timeout: 15_000,
      timeoutMsg: 'Recovery Phrase panel did not mount',
    });

    await clickRecoveryConsentCheckbox();
    await clickText('Save Recovery Phrase', 10_000);

    await browser.waitUntil(async () => await textExists('Recovery phrase saved'), {
      timeout: 20_000,
      timeoutMsg: 'wallet setup success message never rendered',
    });

    await browser.waitUntil(
      async () => {
        const status = await callOpenhumanRpc<{ result: { configured: boolean } }>(
          'openhuman.wallet_status',
          {}
        );
        return status.ok && status.result?.result?.configured === true;
      },
      { timeout: 20_000, timeoutMsg: 'wallet_status never became configured' }
    );

    const walletState = await callOpenhumanRpc<{
      result: { content_utf8: string; truncated: boolean };
    }>('openhuman.test_support_read_workspace_file', {
      rel_path: 'state/wallet-state.json',
      max_bytes: 131_072,
    });
    expect(walletState.ok).toBe(true);
    const content = walletState.result?.result?.content_utf8 ?? '';
    expect(content).toContain('"consentGranted": true');
    expect(content).toContain('"source": "generated"');
    expect(content).toContain('"chain": "evm"');
    expect(content).toContain('"chain": "btc"');
    expect(content).toContain('"chain": "solana"');
    expect(content).toContain('"chain": "tron"');
  });

  it('routes a real chat turn through the crypto agent and creates a prepared wallet quote', async () => {
    clearRequestLog();
    setMockBehavior('llmForcedResponses', JSON.stringify(FORCED_RESPONSES));
    setMockBehavior('llmStreamChunkDelayMs', '10');

    await navigateViaHash('/chat');
    await browser.waitUntil(async () => await textExists('Threads'), {
      timeout: 15_000,
      timeoutMsg: 'Conversations did not mount',
    });
    expect(await clickByTitle('New thread', 8_000)).toBe(true);

    const threadId = (await browser.waitUntil(async () => await getSelectedThreadId(), {
      timeout: 8_000,
      timeoutMsg: 'thread.selectedThreadId never populated',
    })) as string;
    expect(typeof threadId).toBe('string');

    await typeIntoComposer(WALLET_PROMPT);
    expect(
      await browser.waitUntil(async () => await clickSend(), {
        timeout: 5_000,
        timeoutMsg: 'Send button never enabled',
      })
    ).toBe(true);

    await browser.waitUntil(async () => await textExists(CANARY), {
      timeout: 30_000,
      timeoutMsg: 'wallet chat flow never rendered the final canary',
    });

    await browser.waitUntil(
      async () => {
        const quotes = await callOpenhumanRpc<{
          result: {
            count: number;
            quotes: Array<{ toAddress: string; amountRaw: string; status: string; kind: string }>;
          };
        }>('openhuman.test_support_wallet_prepared_quotes', {});
        if (!quotes.ok) return false;
        return (quotes.result?.result?.quotes ?? []).some(
          quote =>
            quote.toAddress === JOHN_ADDRESS &&
            quote.amountRaw === '5000000000000000000' &&
            quote.status === 'awaiting_confirmation' &&
            quote.kind === 'native_transfer'
        );
      },
      {
        timeout: 15_000,
        timeoutMsg: 'prepared wallet quote never appeared in Rust-side introspection',
      }
    );

    const log = getRequestLog() as Array<{ method: string; url: string }>;
    const llmHits = log.filter(
      entry => entry.method === 'POST' && entry.url.includes('/openai/v1/chat/completions')
    );
    expect(llmHits.length).toBeGreaterThanOrEqual(4);

    const relPath = `memory/conversations/threads/${hexEncodeThreadId(threadId)}.jsonl`;
    const read = await callOpenhumanRpc<{ result: { content_utf8: string } }>(
      'openhuman.test_support_read_workspace_file',
      { rel_path: relPath, max_bytes: 131_072 }
    );
    expect(read.ok).toBe(true);
    const threadContent = read.result?.result?.content_utf8 ?? '';
    expect(threadContent).toContain(CANARY);
    expect(threadContent).toContain(WALLET_PROMPT);
  });
});
