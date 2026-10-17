// @ts-nocheck
/**
 * Composio GitHub tools — tags query param flow (spec GT).
 *
 * Verifies that when the core calls
 *   GET /agent-integrations/composio/tools?toolkits=github&tags=<csv>
 * the mock backend receives the correct query params and returns a
 * tag-filtered tool list can be passed to the Composio execution endpoint.
 *
 * Scenarios:
 *   GT.1 — composio.list_tools RPC with toolkits=["github"] and tags=["stars"]
 *           forwards ?toolkits=github&tags=stars and returns only starred tools.
 *   GT.2 — tags with OR semantics: tags=["stars","repos"] returns the union.
 *   GT.3 — non-GitHub toolkit ignores tags (tags stripped before forwarding).
 *   GT.4 — a stars-filtered action can be executed through the public RPC.
 */
import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { resetApp } from '../helpers/reset-app';
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
  // Use actions in TinyConnectors' curated GitHub catalog. Backend listings
  // apply the user's scope policy, which intentionally hides uncurated slugs.
  makeTool('GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER', 'List repositories'),
  makeTool('GITHUB_GET_A_REPOSITORY', 'Get a repository'),
  makeTool('GITHUB_LIST_REPOSITORY_COLLABORATORS', 'List repository collaborators'),
];

const REPOS_TOOLS = [
  makeTool('GITHUB_SEARCH_REPOSITORIES', 'Search repositories'),
  makeTool('GITHUB_LIST_REPOSITORY_ISSUES', 'List repository issues'),
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
    // Outcome with logs serializes as { result: value, logs: [...] };
    // without logs it returns value directly. Unwrap both shapes.
    const raw = result.result as any;
    const tools: Array<{ function: { name: string } }> = raw?.result?.tools ?? raw?.tools ?? [];

    console.log(`${LOG_PREFIX} GT.1: received ${tools.length} tool(s)`);
    expect(tools.length).toBe(STARS_TOOLS.length);

    const names = tools.map(t => t.function.name);
    expect(names).toContain('GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER');
    expect(names).toContain('GITHUB_GET_A_REPOSITORY');
    // repos-only tool must not appear
    expect(names).not.toContain('GITHUB_SEARCH_REPOSITORIES');

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
    expect(names).toContain('GITHUB_GET_A_REPOSITORY');
    expect(names).toContain('GITHUB_SEARCH_REPOSITORIES');

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
    expect(names).not.toContain('GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER');

    console.log(`${LOG_PREFIX} GT.3: PASSED — mock URL: ${toolsHit!.url}`);
  });

  // ── GT.4 — Agent prompt triggers starred-repos tool call ─────────────────

  it('GT.4 — executes an action returned by the stars-filtered tool list', async function () {
    this.timeout(60_000);
    console.log(`${LOG_PREFIX} GT.4: begin`);

    clearRequestLog();
    resetMockBehavior();
    seedGitHubState();

    const STARRED_REPOS = [
      { name: 'awesome-rust', full_name: 'rust-lang/awesome-rust', stargazers_count: 12000 },
      { name: 'tokio', full_name: 'tokio-rs/tokio', stargazers_count: 24000 },
    ];
    setMockBehavior(
      'composioExecuteResponse_GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER',
      JSON.stringify({ repositories: STARRED_REPOS })
    );

    const listed = await callOpenhumanRpc('openhuman.composio_list_tools', {
      toolkits: ['github'],
      tags: ['stars'],
    });
    expect(listed.ok).toBe(true);
    const listedPayload = listed.result as any;
    const tools = listedPayload?.result?.tools ?? listedPayload?.tools ?? [];
    const selectedAction = tools.find(
      (tool: any) => tool.function?.name === 'GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER'
    )?.function?.name;
    expect(selectedAction).toBe('GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER');

    const executed = await callOpenhumanRpc('openhuman.composio_execute', {
      tool: selectedAction,
      arguments: { per_page: 30 },
      connection_id: 'conn-github',
    });
    expect(executed.ok).toBe(true);
    const executeResult = (executed.result as any)?.result ?? executed.result;
    expect(executeResult?.successful).toBe(true);
    expect(executeResult?.data?.repositories).toEqual(STARRED_REPOS);

    const log = getRequestLog() as Array<{ method: string; url: string; body: string }>;
    const listHit = log.find(
      r => r.method === 'GET' && r.url.includes('/agent-integrations/composio/tools')
    );
    expect(listHit?.url).toContain('tags=stars');
    const execHit = log.find(
      r => r.method === 'POST' && r.url.includes('/agent-integrations/composio/execute')
    );
    expect(execHit).toBeDefined();
    const executeBody = JSON.parse(execHit!.body);
    expect(executeBody).toMatchObject({
      tool: selectedAction,
      arguments: { per_page: 30 },
      connectionId: 'conn-github',
    });
    console.log(`${LOG_PREFIX} GT.4: composio execute confirmed — ${execHit!.url}`);

    console.log(`${LOG_PREFIX} GT.4: PASSED`);
  });
});
