// @ts-nocheck
/**
 * Composio GitHub tools — tags query param flow (spec GT).
 *
 * Verifies that when the core calls
 *   GET /agent-integrations/composio/tools?toolkits=github&tags=<csv>
 * the mock backend receives the correct query params and returns a
 * tag-filtered tool list that the agent prompt and tool-call flow
 * correctly use.
 *
 * Scenarios:
 *   GT.1 — composio.list_tools RPC with toolkits=["github"] and tags=["stars"]
 *           forwards ?toolkits=github&tags=stars and returns only starred tools.
 *   GT.2 — tags with OR semantics: tags=["stars","repos"] returns the union.
 *   GT.3 — non-GitHub toolkit ignores tags (tags stripped before forwarding).
 *   GT.4 — agent prompt "list my starred GitHub repos" triggers a tool call
 *           that uses the stars tag, and the final reply mentions the repo.
 */
import { waitForApp } from '../helpers/app-helpers';
import {
  clickByTitle,
  clickSend,
  getSelectedThreadId,
  typeIntoComposer,
  waitForAssistantReplyContaining,
  waitForSocketConnected,
} from '../helpers/chat-harness';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import {
  clearRequestLog,
  getRequestLog,
  resetMockBehavior,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

const LOG_PREFIX = '[ComposioGitHubToolsTags]';
const USER_ID = 'e2e-composio-github-tools-tags';

// ── Fixtures ──────────────────────────────────────────────────────────────────

/** A minimal OpenAI function-calling tool object for a given action slug. */
function makeTool(name: string, description = '') {
  return {
    type: 'function',
    function: {
      name,
      description: description || name,
      parameters: { type: 'object', properties: {} },
    },
  };
}

const STARS_TOOLS = [
  makeTool('GITHUB_LIST_REPOSITORIES_STARRED_BY_THE_AUTHENTICATED_USER', 'List starred repos'),
  makeTool('GITHUB_LIST_STARGAZERS', 'List stargazers'),
  makeTool('GITHUB_STAR_A_REPOSITORY_FOR_THE_AUTHENTICATED_USER', 'Star a repo'),
];

const REPOS_TOOLS = [
  makeTool('GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER', 'List repos'),
  makeTool('GITHUB_CREATE_A_REPOSITORY_FOR_THE_AUTHENTICATED_USER', 'Create a repo'),
];

// ── Seed helper ───────────────────────────────────────────────────────────────

function seedGitHubState(): void {
  setMockBehavior('composioToolkits', JSON.stringify(['github', 'gmail']));
  setMockBehavior(
    'composioConnections',
    JSON.stringify([{ id: 'conn-github', toolkit: 'github', status: 'ACTIVE' }])
  );
  setMockBehavior('composioToolsByTag_stars', JSON.stringify(STARS_TOOLS));
  setMockBehavior('composioToolsByTag_repos', JSON.stringify(REPOS_TOOLS));
}

// ── Chat helper ───────────────────────────────────────────────────────────────

async function navigateChatAndSend(prompt: string): Promise<void> {
  await navigateViaHash('/chat');
  await browser.waitUntil(
    async () => {
      if (await getSelectedThreadId()) return true;
      if (await textExists('No messages yet')) return true;
      return textExists('Type a message');
    },
    { timeout: 15_000, timeoutMsg: 'Chat surface did not mount' }
  );
  if (!(await getSelectedThreadId())) {
    const clicked =
      (await clickByTitle('New thread', 8_000)) ||
      (await clickByTitle('New thread (/new)', 3_000)) ||
      (await browser.execute(() => {
        const btn = Array.from(document.querySelectorAll('button')).find(
          b => (b.textContent ?? '').trim() === 'New'
        ) as HTMLButtonElement | undefined;
        if (!btn) return false;
        btn.click();
        return true;
      }));
    expect(clicked).toBe(true);
    await browser.waitUntil(async () => await getSelectedThreadId(), {
      timeout: 8_000,
      timeoutMsg: 'thread.selectedThreadId never populated',
    });
  }
  await typeIntoComposer(prompt);
  const socketReady = await waitForSocketConnected(30_000);
  if (!socketReady) {
    console.warn(`${LOG_PREFIX} socket did not connect within 30s — send may fail`);
  }
  expect(
    await browser.waitUntil(async () => await clickSend(), {
      timeout: 15_000,
      timeoutMsg: 'Send button never enabled',
    })
  ).toBe(true);
  console.log(`${LOG_PREFIX} Sent prompt: "${prompt.slice(0, 60)}"`);
}

// ── Suite ─────────────────────────────────────────────────────────────────────

describe('Composio GitHub tools — tags query param flow', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    console.log(`${LOG_PREFIX} Starting mock server and resetting app`);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
    console.log(`${LOG_PREFIX} Suite setup complete`);
  });

  after(async () => {
    resetMockBehavior();
    await stopMockServer();
    console.log(`${LOG_PREFIX} Suite teardown complete`);
  });

  // ── GT.1 — Single tag forwarded and filtered ─────────────────────────────

  it('GT.1 — composio.list_tools with tags=["stars"] returns only starred-tools and hits mock with correct query', async function () {
    this.timeout(60_000);
    console.log(`${LOG_PREFIX} GT.1: begin`);

    clearRequestLog();
    resetMockBehavior();
    seedGitHubState();

    const result = await callOpenhumanRpc('openhuman.composio_list_tools', {
      toolkits: ['github'],
      tags: ['stars'],
    });

    expect(result.ok).toBe(true);
    // RpcOutcome with logs serializes as { result: value, logs: [...] };
    // without logs it returns value directly. Unwrap both shapes.
    const raw = result.result as any;
    const tools: Array<{ function: { name: string } }> = raw?.result?.tools ?? raw?.tools ?? [];

    console.log(`${LOG_PREFIX} GT.1: received ${tools.length} tool(s)`);
    expect(tools.length).toBe(STARS_TOOLS.length);

    const names = tools.map(t => t.function.name);
    expect(names).toContain('GITHUB_LIST_REPOSITORIES_STARRED_BY_THE_AUTHENTICATED_USER');
    expect(names).toContain('GITHUB_STAR_A_REPOSITORY_FOR_THE_AUTHENTICATED_USER');
    // repos-only tool must not appear
    expect(names).not.toContain('GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER');

    // Verify the mock received the correct query params.
    const log = getRequestLog() as Array<{ method: string; url: string }>;
    const toolsHit = log.find(
      r => r.method === 'GET' && r.url.includes('/agent-integrations/composio/tools')
    );
    expect(toolsHit).toBeDefined();
    expect(toolsHit!.url).toContain('toolkits=github');
    expect(toolsHit!.url).toContain('tags=stars');

    console.log(`${LOG_PREFIX} GT.1: PASSED — mock hit: ${toolsHit!.url}`);
  });

  // ── GT.2 — OR semantics across multiple tags ─────────────────────────────

  it('GT.2 — tags=["stars","repos"] returns the union of both tag sets', async function () {
    this.timeout(60_000);
    console.log(`${LOG_PREFIX} GT.2: begin`);

    clearRequestLog();
    resetMockBehavior();
    seedGitHubState();

    const result = await callOpenhumanRpc('openhuman.composio_list_tools', {
      toolkits: ['github'],
      tags: ['stars', 'repos'],
    });

    expect(result.ok).toBe(true);
    const raw = result.result as any;
    const tools: Array<{ function: { name: string } }> = raw?.result?.tools ?? raw?.tools ?? [];

    const names = tools.map(t => t.function.name);
    const expectedCount = STARS_TOOLS.length + REPOS_TOOLS.length;

    console.log(`${LOG_PREFIX} GT.2: received ${tools.length} tool(s), expected ${expectedCount}`);
    expect(tools.length).toBe(expectedCount);

    // Both tag sets should be present.
    expect(names).toContain('GITHUB_LIST_REPOSITORIES_STARRED_BY_THE_AUTHENTICATED_USER');
    expect(names).toContain('GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER');

    // Verify the mock URL carries both tags (comma-separated).
    const log = getRequestLog() as Array<{ method: string; url: string }>;
    const toolsHit = log.find(
      r => r.method === 'GET' && r.url.includes('/agent-integrations/composio/tools')
    );
    expect(toolsHit).toBeDefined();
    expect(toolsHit!.url).toContain('tags=stars');
    expect(toolsHit!.url).toContain('repos');

    console.log(`${LOG_PREFIX} GT.2: PASSED`);
  });

  // ── GT.3 — Non-GitHub toolkit strips tags ────────────────────────────────

  it('GT.3 — tags are ignored when toolkit is not github', async function () {
    this.timeout(60_000);
    console.log(`${LOG_PREFIX} GT.3: begin`);

    clearRequestLog();
    resetMockBehavior();
    setMockBehavior('composioToolkits', JSON.stringify(['gmail']));
    setMockBehavior(
      'composioConnections',
      JSON.stringify([{ id: 'conn-gmail', toolkit: 'gmail', status: 'ACTIVE' }])
    );
    // Seed some gmail tools so the endpoint returns something.
    const GMAIL_TOOLS = [
      makeTool('GMAIL_GET_MAIL', 'Get mail'),
      makeTool('GMAIL_SEND_EMAIL', 'Send email'),
    ];
    setMockBehavior('composioTools', JSON.stringify(GMAIL_TOOLS));
    // tags knob for "stars" — must NOT appear in gmail response.
    setMockBehavior('composioToolsByTag_stars', JSON.stringify(STARS_TOOLS));

    const result = await callOpenhumanRpc('openhuman.composio_list_tools', {
      toolkits: ['gmail'],
      tags: ['stars'],
    });

    expect(result.ok).toBe(true);
    const raw = result.result as any;
    const tools: Array<{ function: { name: string } }> = raw?.result?.tools ?? raw?.tools ?? [];
    const names = tools.map(t => t.function.name);

    console.log(`${LOG_PREFIX} GT.3: received ${tools.length} tool(s): ${names.join(', ')}`);

    // Tags must have been stripped — mock must NOT receive ?tags= for gmail.
    const log = getRequestLog() as Array<{ method: string; url: string }>;
    const toolsHit = log.find(
      r => r.method === 'GET' && r.url.includes('/agent-integrations/composio/tools')
    );
    expect(toolsHit).toBeDefined();
    expect(toolsHit!.url).not.toContain('tags=');
    // Stars tools must not appear in the gmail response.
    expect(names).not.toContain('GITHUB_LIST_REPOSITORIES_STARRED_BY_THE_AUTHENTICATED_USER');

    console.log(`${LOG_PREFIX} GT.3: PASSED — mock URL: ${toolsHit!.url}`);
  });

  // ── GT.4 — Agent prompt triggers starred-repos tool call ─────────────────

  it('GT.4 — "list my starred GitHub repos" prompt triggers stars-tagged tool call and reply lists repo', async function () {
    this.timeout(120_000);
    console.log(`${LOG_PREFIX} GT.4: begin`);

    clearRequestLog();
    resetMockBehavior();
    seedGitHubState();

    const STARRED_REPOS = [
      { name: 'awesome-rust', full_name: 'rust-lang/awesome-rust', stargazers_count: 12000 },
      { name: 'tokio', full_name: 'tokio-rs/tokio', stargazers_count: 24000 },
    ];
    setMockBehavior(
      'composioExecuteResponse_GITHUB_LIST_REPOSITORIES_STARRED_BY_THE_AUTHENTICATED_USER',
      JSON.stringify({ repositories: STARRED_REPOS })
    );

    const CANARY = 'canary-github-stars-a1b2c3';
    const FORCED = [
      {
        content: '',
        toolCalls: [
          {
            id: 'call_github_stars_1',
            name: 'GITHUB_LIST_REPOSITORIES_STARRED_BY_THE_AUTHENTICATED_USER',
            arguments: JSON.stringify({ per_page: 30 }),
          },
        ],
      },
      { content: `Your starred repos: awesome-rust, tokio. ${CANARY}` },
    ];
    setMockBehavior('llmForcedResponses', JSON.stringify(FORCED));
    setMockBehavior('llmStreamChunkDelayMs', '10');

    await navigateChatAndSend('list my starred GitHub repos');

    await browser.waitUntil(async () => await textExists(CANARY), {
      timeout: 60_000,
      timeoutMsg: `GT.4: final reply canary "${CANARY}" never appeared`,
    });
    expect(await waitForAssistantReplyContaining('awesome-rust', { logPrefix: LOG_PREFIX })).toBe(
      true
    );

    const log = getRequestLog() as Array<{ method: string; url: string }>;
    const llmHits = log.filter(r => r.method === 'POST' && r.url.includes('/chat/completions'));
    console.log(`${LOG_PREFIX} GT.4: ${llmHits.length} LLM completion request(s)`);
    expect(llmHits.length).toBeGreaterThanOrEqual(2);

    // Verify the composio execute was called for the forced tool call.
    const execHit = log.find(
      r => r.method === 'POST' && r.url.includes('/agent-integrations/composio/execute')
    );
    expect(execHit).toBeDefined();
    console.log(`${LOG_PREFIX} GT.4: composio execute confirmed — ${execHit!.url}`);

    console.log(`${LOG_PREFIX} GT.4: PASSED`);
  });
});
