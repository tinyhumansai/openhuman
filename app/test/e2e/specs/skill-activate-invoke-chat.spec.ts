/**
 * E2E: activate a skill → invoke it from chat → see the result (issue #4273, AC5).
 *
 * This is the headline lifecycle the Skills page promises but that no spec
 * previously covered end to end: a user activates a skill (here a Composio
 * toolkit connection), then asks the agent to do something in chat, the agent
 * routes through the skills/Composio tool surface, and the result renders in
 * the conversation.
 *
 * Strategy:
 *  - Seed an ACTIVE Composio connection so the skill is "activated".
 *  - Force two LLM turns via the mock: turn 1 emits a `composio_execute`
 *    tool call, turn 2 returns the final answer once the tool result is fed
 *    back. This mirrors `chat-tool-call-flow.spec.ts` but exercises the skill
 *    (Composio) tool path instead of `web_fetch`.
 *  - Assert the tool call reached the Composio execute endpoint AND the final
 *    answer renders.
 *  - Assert the whole chain completes within the 60s latency target (AC3).
 */
import { waitForApp } from '../helpers/app-helpers';
import {
  chatMounted,
  clickByTitle,
  clickSend,
  getSelectedThreadId,
  typeIntoComposer,
  waitForAssistantReplyContaining,
  waitForSocketConnected,
  waitForToolCallInMockLog,
} from '../helpers/chat-harness';
import { seedComposioConnection, seedComposioToolkits } from '../helpers/composio-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import {
  clearRequestLog,
  resetMockBehavior,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

const LOG_PREFIX = '[skill-activate-invoke-chat]';
const USER_ID = 'e2e-skill-activate-invoke-chat';
const TOOLKIT_SLUG = 'gmail';
const COMPOSIO_ACTION = 'GMAIL_FETCH_EMAILS';
const PROMPT = 'Use my connected Gmail to fetch my latest emails.';
const CANARY_FINAL = 'canary-skill-invoked-e7f1a9';

// Turn 1: the agent calls the Composio skill tool. Turn 2: the final answer
// after the core feeds the tool result back to the LLM.
const FORCED_RESPONSES = [
  {
    content: '',
    toolCalls: [
      {
        id: 'call_composio_execute_1',
        name: 'composio_execute',
        arguments: JSON.stringify({ tool: COMPOSIO_ACTION, arguments: {} }),
      },
    ],
  },
  { content: `Here are your latest emails: ${CANARY_FINAL}` },
];

describe('Skill activate → invoke from chat', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    await startMockServer();
    // Activate the skill: an ACTIVE Composio connection the agent can route to.
    seedComposioToolkits([TOOLKIT_SLUG]);
    seedComposioConnection(TOOLKIT_SLUG, 'ACTIVE', 'c-gmail-1');
    await waitForApp();
    await resetApp(USER_ID);

    setMockBehavior('llmForcedResponses', JSON.stringify(FORCED_RESPONSES));
    setMockBehavior('llmStreamChunkDelayMs', '10');
    clearRequestLog();
    console.log(`${LOG_PREFIX} setup complete — skill activated, forced responses configured`);
  });

  after(async () => {
    setMockBehavior('llmForcedResponses', '');
    setMockBehavior('llmStreamChunkDelayMs', '');
    resetMockBehavior();
    await stopMockServer();
  });

  it('routes a chat request through the activated skill and renders the result within 60s', async function () {
    this.timeout(90_000);

    await navigateViaHash('/chat');
    await browser.waitUntil(async () => await chatMounted(), {
      timeout: 15_000,
      timeoutMsg: 'chat did not mount',
    });
    expect(await clickByTitle('New thread', 8_000)).toBe(true);

    const threadId = (await browser.waitUntil(async () => await getSelectedThreadId(), {
      timeout: 8_000,
      timeoutMsg: 'thread.selectedThreadId never populated',
    })) as string;
    expect(typeof threadId).toBe('string');

    await typeIntoComposer(PROMPT);
    const socketReady = await waitForSocketConnected(30_000);
    if (!socketReady) {
      console.warn(`${LOG_PREFIX} socket did not connect within 30s — send may fail`);
    }

    // Start the latency clock at send so we can assert the AC3 budget on the
    // full activate→invoke→result chain.
    const startedAt = Date.now();
    expect(
      await browser.waitUntil(async () => await clickSend(), {
        timeout: 5_000,
        timeoutMsg: 'Send button never enabled',
      })
    ).toBe(true);

    // The agent should route to the Composio skill execute endpoint. This is a
    // soft signal (the final answer below is the hard oracle) so a faster-than-
    // poll turn doesn't flake the test.
    const toolHit = await waitForToolCallInMockLog(COMPOSIO_ACTION, {
      source: 'composio',
      timeoutMs: 45_000,
      logPrefix: LOG_PREFIX,
    });
    // Hard assertion: the whole point of this spec is the activate→invoke
    // routing guarantee, so the agent MUST hit the Composio execute endpoint —
    // a missing call is a failure, not a warning (PR #4288 review).
    expect(toolHit).toBeDefined();

    // Hard oracle: the skill's result renders in the conversation.
    const replied = await waitForAssistantReplyContaining(CANARY_FINAL, {
      timeoutMs: 50_000,
      logPrefix: LOG_PREFIX,
    });
    expect(replied).toBe(true);

    const elapsedMs = Date.now() - startedAt;
    console.log(`${LOG_PREFIX} activate→invoke→result completed in ${elapsedMs}ms`);
    // AC3: complex skill tool chains complete within 60 seconds.
    expect(elapsedMs).toBeLessThan(60_000);
  });
});
