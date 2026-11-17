/**
 * MCP page — full lifecycle e2e tests over its three tabs.
 *
 * Covers: the server rows → manage detail view → connect → run a tool →
 * uninstall; the mcp.json editor → declare a server → it appears in the rows;
 * the registry → a hosted server is added in one click, any other row opens
 * the server's page. All RPC calls are mocked via page.route so no running
 * core is required.
 */
import { expect, type Page, test } from '@playwright/test';

import packageJson from '../../../package.json' with { type: 'json' };

// Derive from the build's real version, never hardcode. `update_version` feeds
// the bootCheck version-match gate; a stale literal makes the mock mismatch the
// app build, leaving BootCheckGate in "outdated" so `#root` never renders and
// the whole spec times out (the 0.57.18→0.57.19 bump blanked it this way).
const APP_VERSION = packageJson.version;

// ---------------------------------------------------------------------------
// Mock data
// ---------------------------------------------------------------------------

const REGISTRY_SERVERS = [
  {
    qualified_name: 'io.github.test/memory-server',
    display_name: 'Memory Server',
    description: 'A test MCP server for memory operations',
    icon_url: null,
    use_count: 1200,
    is_deployed: false,
    source: 'mcp_official',
  },
  {
    qualified_name: 'io.github.test/github-tools',
    display_name: 'GitHub Tools',
    description: 'MCP server for GitHub API integration',
    icon_url: null,
    use_count: 5600,
    is_deployed: true,
    source: 'mcp_official',
  },
  {
    qualified_name: 'io.github.test/notion-connector',
    display_name: 'Notion Connector',
    description: 'Connect to Notion workspaces via MCP',
    icon_url: null,
    use_count: 980,
    is_deployed: false,
    source: 'mcp_official',
  },
  {
    qualified_name: 'io.github.test/open-hosted',
    display_name: 'Open Hosted',
    description: 'A hosted MCP server that needs no setup',
    icon_url: null,
    use_count: 300,
    is_deployed: true,
    source: 'mcp_official',
  },
  {
    qualified_name: 'io.github.test/tenant-hosted',
    display_name: 'Tenant Hosted',
    description: 'A hosted MCP server with a per-tenant endpoint',
    icon_url: null,
    use_count: 120,
    is_deployed: true,
    source: 'mcp_official',
  },
];

const OPEN_HOSTED = 'io.github.test/open-hosted';

/** What `registry_get` answers for the hosted directory rows. */
const REGISTRY_DETAILS: Record<string, { type: string; deployment_url: string }[]> = {
  [OPEN_HOSTED]: [{ type: 'http', deployment_url: 'https://open-hosted.test/mcp' }],
  'io.github.test/github-tools': [
    { type: 'http', deployment_url: 'https://github-tools.test/mcp' },
  ],
  'io.github.test/tenant-hosted': [
    { type: 'http', deployment_url: 'https://{tenant}.tenant-hosted.test/mcp' },
  ],
};

function makeInstalledServer(overrides: Partial<typeof INSTALLED_DEFAULT> = {}) {
  return { ...INSTALLED_DEFAULT, ...overrides };
}

const INSTALLED_DEFAULT: {
  server_id: string;
  qualified_name: string;
  display_name: string;
  description?: string;
  command_kind: string;
  command: string;
  args: string[];
  env_keys: string[];
  installed_at: number;
  transport?: { kind: 'stdio' } | { kind: 'http_remote'; url: string };
  enabled: boolean;
} = {
  server_id: 'srv_installed_1',
  qualified_name: 'io.github.test/memory-server',
  display_name: 'Memory Server',
  description: 'A test MCP server for memory operations',
  command_kind: 'node',
  command: 'npx',
  args: ['-y', '@modelcontextprotocol/server-memory'],
  env_keys: [],
  installed_at: 1700000000,
  enabled: true,
};

const STATUS_CONNECTED = {
  server_id: 'srv_installed_1',
  qualified_name: 'io.github.test/memory-server',
  display_name: 'Memory Server',
  status: 'connected' as const,
  tool_count: 5,
};

// Tools a connected server exposes — returned by the mocked connect so the
// detail's tool list (and the execution playground) have something to drive.
const MOCK_TOOLS = [
  { name: 'create_memory', description: 'Create a memory', input_schema: {} },
  { name: 'list_memories', description: 'List all memories', input_schema: {} },
];

/** What the core's `config_get` renders: the dial plus credential names. */
function renderDoc(state: MockState) {
  const out: Record<string, unknown> = {};
  for (const s of [...state.installed].sort((a, b) =>
    a.qualified_name.localeCompare(b.qualified_name)
  )) {
    out[s.qualified_name] = {
      ...(s.transport?.kind === 'http_remote' ? { url: s.transport.url } : { command: s.command }),
      ...(s.args.length ? { args: s.args } : {}),
      ...(s.env_keys.length ? { envKeys: s.env_keys } : {}),
      authConfigured: s.env_keys.length > 0,
    };
  }
  return out;
}

// ---------------------------------------------------------------------------
// RPC mock layer — mutable state so tests can drive lifecycle transitions
// ---------------------------------------------------------------------------

interface MockState {
  installed: (typeof INSTALLED_DEFAULT)[];
  // `status` is broadened to a string so tests can seed non-connected states
  // (e.g. `error`) that keep the status poll active.
  statuses: Array<Omit<typeof STATUS_CONNECTED, 'status'> & { status: string }>;
}

function rpcOk(id: number, result: unknown) {
  return {
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({ jsonrpc: '2.0', id, result }),
  };
}

function rpcError(id: number, message: string) {
  return {
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({ jsonrpc: '2.0', id, error: { code: -32000, message } }),
  };
}

async function setupMockRpc(page: Page, state: MockState) {
  await page.route('**/rpc', async (route, request) => {
    const body = JSON.parse(request.postData() || '{}');
    const method: string = body.method;
    const id: number = body.id;
    const params = body.params ?? {};

    switch (method) {
      case 'openhuman.update_version':
        return route.fulfill(
          rpcOk(id, {
            result: {
              version: APP_VERSION,
              target_triple: 'x86_64-apple-darwin',
              asset_prefix: '',
            },
          })
        );

      case 'openhuman.app_state_snapshot':
        return route.fulfill(
          rpcOk(id, {
            result: {
              auth: { isAuthenticated: true, userId: 'pw-mcp-user', user: null, profileId: null },
              sessionToken: 'fake-session-token',
              currentUser: { _id: 'pw-mcp-user', displayName: 'Test User' },
              onboardingCompleted: true,
              chatOnboardingCompleted: true,
              analyticsEnabled: false,
              meetAutoOrchestratorHandoff: false,
              localState: {},
              keyringStatus: { isUnlocked: true, hasPassphrase: false },
              runtime: {
                localAi: { enabled: false },
                autocomplete: { enabled: false },
                service: { running: false },
              },
            },
          })
        );

      // ---- MCP registry ----
      case 'openhuman.mcp_clients_registry_search': {
        const query = (params.query ?? '').toLowerCase();
        const filtered = query
          ? REGISTRY_SERVERS.filter(
              s =>
                s.display_name.toLowerCase().includes(query) ||
                s.qualified_name.toLowerCase().includes(query)
            )
          : REGISTRY_SERVERS;
        return route.fulfill(rpcOk(id, { servers: filtered, page: 1, total_pages: 1 }));
      }

      case 'openhuman.mcp_clients_registry_get': {
        const server = REGISTRY_SERVERS.find(s => s.qualified_name === params.qualified_name);
        const connections = REGISTRY_DETAILS[params.qualified_name];
        if (!server || !connections) {
          return route.fulfill(rpcError(id, `server not found: ${params.qualified_name}`));
        }
        return route.fulfill(
          rpcOk(id, { server: { ...server, connections, required_env_keys: [] } })
        );
      }

      // ---- Installed servers (mutable) ----
      case 'openhuman.mcp_clients_installed_list':
        return route.fulfill(rpcOk(id, { installed: state.installed }));

      case 'openhuman.mcp_clients_status':
        return route.fulfill(rpcOk(id, { servers: state.statuses }));

      // ---- mcp.json (the only way a server is added or removed) ----
      case 'openhuman.mcp_clients_config_get':
        return route.fulfill(rpcOk(id, { mcpServers: renderDoc(state) }));

      case 'openhuman.mcp_clients_config_set': {
        const declared = (params.mcpServers ?? {}) as Record<string, Record<string, unknown>>;
        const names = new Set(Object.keys(declared));
        const removed = state.installed
          .filter(s => !names.has(s.qualified_name))
          .map(s => s.qualified_name);
        state.installed = state.installed.filter(s => names.has(s.qualified_name));
        state.statuses = state.statuses.filter(s => !removed.includes(s.qualified_name));
        const added: string[] = [];
        const updated: string[] = [];
        for (const [name, entry] of Object.entries(declared)) {
          const current = state.installed.find(s => s.qualified_name === name);
          if (current) {
            current.command = (entry.command as string) ?? '';
            current.args = (entry.args as string[]) ?? [];
            updated.push(name);
            continue;
          }
          if (!entry.command && !entry.url) {
            return route.fulfill(
              rpcError(id, `\`${name}\` needs a \`url\` (hosted) or a \`command\` (run locally)`)
            );
          }
          // The store cannot carry a working directory; the real core refuses
          // the field by name so the user can find it in the text.
          if ('cwd' in entry) {
            return route.fulfill(
              rpcError(id, `\`${name}\` has a \`cwd\` field this host doesn't understand`)
            );
          }
          const credentials = (entry.env ?? entry.headers ?? {}) as Record<string, string>;
          state.installed.push(
            makeInstalledServer({
              server_id: `srv_${name}`,
              qualified_name: name,
              display_name: name,
              description: undefined,
              command: (entry.command as string) ?? '',
              args: (entry.args as string[]) ?? [],
              env_keys: Object.keys(credentials),
              transport: entry.url
                ? { kind: 'http_remote', url: entry.url as string }
                : { kind: 'stdio' },
            })
          );
          added.push(name);
        }
        return route.fulfill(rpcOk(id, { mcpServers: renderDoc(state), added, updated, removed }));
      }

      // Auth probe for the upfront connect modal — these test servers need no
      // credentials, so report `none` and the modal shows a single Connect button.
      case 'openhuman.mcp_clients_detect_auth': {
        // A hosted server with nothing stored asks for browser sign-in, except
        // the open directory server; the rest need no credentials.
        const inst = state.installed.find(s => s.server_id === params.server_id);
        const oauth =
          inst?.transport?.kind === 'http_remote' &&
          inst.env_keys.length === 0 &&
          inst.qualified_name !== OPEN_HOSTED;
        return route.fulfill(
          rpcOk(
            id,
            oauth ? { kind: 'oauth', grant_types: ['authorization_code'] } : { kind: 'none' }
          )
        );
      }

      case 'openhuman.mcp_clients_oauth_begin':
        return route.fulfill(rpcOk(id, { authorize_url: 'https://auth.example/authorize' }));

      case 'openhuman.mcp_clients_connect': {
        const sid = params.server_id;
        const inst = state.installed.find(s => s.server_id === sid);
        // Reject unknown server ids so the test can't pass while wired to the
        // wrong server.
        if (!inst) {
          return route.fulfill(rpcError(id, `server not installed: ${sid}`));
        }
        // Mark the server connected for subsequent status polls and hand back
        // its tool list (what `onConnected` feeds into the detail's tool list).
        state.statuses = [
          ...state.statuses.filter(s => s.server_id !== sid),
          {
            server_id: sid,
            qualified_name: inst.qualified_name,
            display_name: inst.display_name,
            status: 'connected',
            tool_count: MOCK_TOOLS.length,
          },
        ];
        return route.fulfill(rpcOk(id, { status: 'connected', tools: MOCK_TOOLS }));
      }

      // Tool execution — what the playground's "Run tool" calls. Unknown tools
      // come back as a tool error so the spec can't pass on a wrong tool name.
      case 'openhuman.mcp_clients_tool_call': {
        const known = MOCK_TOOLS.some(t => t.name === params.tool_name);
        if (!known) {
          return route.fulfill(
            rpcOk(id, { result: `unknown tool: ${params.tool_name}`, is_error: true })
          );
        }
        return route.fulfill(
          rpcOk(id, { result: `ran ${params.tool_name}: memory created id=42`, is_error: false })
        );
      }

      case 'openhuman.mcp_clients_disconnect':
        state.statuses = state.statuses.filter(s => s.server_id !== params.server_id);
        return route.fulfill(rpcOk(id, { status: 'disconnected' }));

      case 'openhuman.mcp_clients_uninstall':
        state.installed = state.installed.filter(s => s.server_id !== params.server_id);
        state.statuses = state.statuses.filter(s => s.server_id !== params.server_id);
        return route.fulfill(rpcOk(id, { success: true }));

      case 'openhuman.mcp_clients_list_tools':
        return route.fulfill(rpcOk(id, { server_id: params.server_id, tools: MOCK_TOOLS }));

      case 'openhuman.mcp_clients_tools':
        return route.fulfill(
          rpcOk(id, {
            tools: [
              { name: 'create_memory', description: 'Create a memory', input_schema: {} },
              { name: 'list_memories', description: 'List all memories', input_schema: {} },
            ],
          })
        );

      default:
        return route.fulfill(rpcOk(id, {}));
    }
  });
}

async function seedLocalStorage(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem('openhuman_core_mode', 'cloud');
    window.localStorage.setItem('openhuman_core_rpc_url', 'http://127.0.0.1:17788/rpc');
    window.localStorage.setItem('openhuman_core_rpc_token', 'test-token');
    window.localStorage.setItem('openhuman:walkthrough_completed', 'true');
    window.localStorage.removeItem('openhuman:walkthrough_pending');
  });
}

async function navigateToMcpTab(page: Page) {
  // Phase 2: /skills → /connections, ?tab=mcp → ?tab=tools (back-compat alias also works)
  await page.goto('/#/connections?tab=tools');
  await page.waitForSelector('#root', { state: 'visible', timeout: 20_000 });
  await page.getByTestId('mcp-page-tab-servers').waitFor({ state: 'visible', timeout: 10_000 });
}

/** A row in the Servers tab, by display name. */
function installedRow(page: Page, name: string) {
  return page.locator('[data-testid="mcp-installed-row"]', {
    has: page.getByRole('button', { name: `Open ${name}` }),
  });
}

// ==========================================================================
// Tests
// ==========================================================================

test.describe('MCP page — Servers tab', () => {
  let state: MockState;

  test.beforeEach(async ({ page }) => {
    state = { installed: [makeInstalledServer()], statuses: [{ ...STATUS_CONNECTED }] };
    await seedLocalStorage(page);
    await setupMockRpc(page, state);
    await navigateToMcpTab(page);
  });

  test('renders the three notations as tabs in the page header and opens on the rows', async ({
    page,
  }) => {
    await expect(page.getByRole('heading', { level: 1, name: 'MCP Servers' })).toBeVisible();
    await expect(page.getByRole('tab', { name: 'Servers' })).toBeVisible();
    await expect(page.getByRole('tab', { name: 'mcp.json' })).toBeVisible();
    await expect(page.getByRole('tab', { name: 'Registry' })).toBeVisible();
    await expect(page.getByTestId('mcp-servers-section')).toBeVisible();
    await expect(installedRow(page, 'Memory Server')).toBeVisible();
  });

  test('displays declared servers with their dial, status and icon controls', async ({ page }) => {
    const row = installedRow(page, 'Memory Server');
    await expect(row).toBeVisible();
    await expect(row).toContainText('npx -y @modelcontextprotocol/server-memory');
    await expect(row.getByTestId('mcp-row-status')).toContainText('Connected');
    await expect(row.getByRole('button', { name: 'Disconnect Memory Server' })).toBeVisible();
    await expect(row.getByRole('button', { name: 'Disable Memory Server' })).toBeVisible();
    await expect(row.getByRole('button', { name: 'Remove Memory Server' })).toBeVisible();
  });

  test('the rows hold nothing from the directory, and nothing installs', async ({ page }) => {
    await expect(page.getByRole('button', { name: /^Install$/ })).toHaveCount(0);
    await expect(page.locator('text=GitHub Tools')).toHaveCount(0);
  });

  test('a connected row lists its tools and runs one from the playground', async ({ page }) => {
    await page.getByRole('button', { name: "Show Memory Server's tools" }).click();
    const tools = page.getByTestId('mcp-row-tools');
    await expect(tools).toBeVisible({ timeout: 5_000 });
    await expect(tools).toContainText('create_memory');
    await tools
      .getByRole('button', { name: 'Open execution playground for create_memory' })
      .click();
    const playground = page.getByRole('dialog');
    await expect(playground.getByText('Run create_memory')).toBeVisible({ timeout: 5_000 });
    await playground.getByRole('button', { name: 'Run tool' }).click();
    await expect(page.getByTestId('mcp-playground-result')).toContainText('memory created id=42', {
      timeout: 10_000,
    });
  });

  test('a row control acts on the server and the row follows', async ({ page }) => {
    await installedRow(page, 'Memory Server')
      .getByRole('button', { name: 'Disconnect Memory Server' })
      .click();
    await expect(
      installedRow(page, 'Memory Server').getByRole('button', { name: 'Connect Memory Server' })
    ).toBeVisible({ timeout: 5_000 });
  });

  test('no Smithery branding visible anywhere', async ({ page }) => {
    await installedRow(page, 'Memory Server').waitFor({ state: 'visible', timeout: 10_000 });
    const bodyText = await page.locator('body').innerText();
    expect(bodyText.toLowerCase()).not.toContain('smithery');
  });

  test('add flow: form → local command with env → the row appears', async ({ page }) => {
    await page.getByTestId('mcp-add-server').click();
    const form = page.getByTestId('mcp-server-form');
    await expect(form).toBeVisible({ timeout: 5_000 });
    await form.getByTestId('mcp-form-name').fill('github');
    await form.getByTestId('mcp-form-command').fill('npx');
    await form.getByTestId('mcp-form-args').fill('-y @modelcontextprotocol/server-github');
    await form.getByLabel('Variable name').fill('GITHUB_TOKEN');
    await form.getByLabel('Value', { exact: true }).fill('ghp_test');
    await form.getByTestId('mcp-form-save').click();

    await expect(form).not.toBeVisible({ timeout: 5_000 });
    const row = installedRow(page, 'github');
    await expect(row).toBeVisible({ timeout: 5_000 });
    await expect(row).toContainText('npx -y @modelcontextprotocol/server-github');
  });

  test('add flow: remote URL with browser sign-in hands off to the connect dialog', async ({
    page,
  }) => {
    await page.getByTestId('mcp-add-server').click();
    const form = page.getByTestId('mcp-server-form');
    await form.getByTestId('mcp-form-name').fill('notion');
    await form.getByTestId('mcp-form-transport').selectOption('http');
    await form.getByTestId('mcp-form-url').fill('https://mcp.notion.com/mcp');
    await form.getByTestId('mcp-form-auth').selectOption('oauth');
    await expect(form.getByTestId('mcp-form-save')).toHaveText('Save & sign in');
    await form.getByTestId('mcp-form-save').click();

    // The connect dialog opens for the new server and offers the sign-in.
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible({ timeout: 5_000 });
    await expect(dialog).toContainText('Connect notion');
    await expect(dialog.getByRole('button', { name: 'Sign in with browser' })).toBeVisible();
  });

  test("edit flow: the row's pencil opens the form prefilled, and saves in place", async ({
    page,
  }) => {
    await page.getByRole('button', { name: 'Edit Memory Server' }).click();
    const form = page.getByTestId('mcp-server-form');
    await expect(form.getByTestId('mcp-form-name')).toHaveValue('io.github.test/memory-server');
    await expect(form.getByTestId('mcp-form-command')).toHaveValue('npx');
    await form
      .getByTestId('mcp-form-args')
      .fill('-y @modelcontextprotocol/server-memory --verbose');
    await form.getByTestId('mcp-form-save').click();
    await expect(form).not.toBeVisible({ timeout: 5_000 });
    await expect(installedRow(page, 'Memory Server')).toContainText('--verbose');
  });
});

test.describe('MCP page — mcp.json tab', () => {
  let state: MockState;

  test.beforeEach(async ({ page }) => {
    state = { installed: [makeInstalledServer()], statuses: [{ ...STATUS_CONNECTED }] };
    await seedLocalStorage(page);
    await setupMockRpc(page, state);
    await navigateToMcpTab(page);
    await page.getByRole('tab', { name: 'mcp.json' }).click();
    await expect(page.getByTestId('mcp-json-editor')).toBeVisible({ timeout: 10_000 });
  });

  test('shows the declared servers as one document, with no credential values', async ({
    page,
  }) => {
    const text = await page.getByTestId('mcp-json-textarea').inputValue();
    expect(text).toContain('"mcpServers"');
    expect(text).toContain('"io.github.test/memory-server"');
    expect(text).toContain('"authConfigured": false');
    await expect(page.getByTestId('mcp-json-save')).toBeDisabled();
  });

  test('declare flow: paste a server block → save → it appears in the rows', async ({ page }) => {
    await page
      .getByTestId('mcp-json-textarea')
      .fill(
        JSON.stringify(
          {
            mcpServers: {
              'io.github.test/memory-server': {
                command: 'npx',
                args: ['-y', '@modelcontextprotocol/server-memory'],
              },
              github: {
                command: 'npx',
                args: ['-y', '@modelcontextprotocol/server-github'],
                env: { GITHUB_TOKEN: 'ghp_test_token_123' },
              },
            },
          },
          null,
          2
        )
      );
    await expect(page.getByTestId('mcp-json-save')).toBeEnabled();
    await page.getByTestId('mcp-json-save').click();

    await expect(page.getByTestId('mcp-json-saved')).toContainText('1 added', { timeout: 5_000 });
    // The buffer is what the core rendered back: names, never the value.
    const text = await page.getByTestId('mcp-json-textarea').inputValue();
    expect(text).toContain('"GITHUB_TOKEN"');
    expect(text).not.toContain('ghp_test_token_123');

    await page.getByRole('tab', { name: 'Servers' }).click();
    await expect(installedRow(page, 'github')).toBeVisible({ timeout: 5_000 });
  });

  test('a document the core refuses shows its reason and keeps the text', async ({ page }) => {
    // Passes the editor's own shape check; only the core knows it cannot
    // carry a `cwd`.
    const text = JSON.stringify({ mcpServers: { bad: { command: 'x', cwd: '/tmp' } } });
    await page.getByTestId('mcp-json-textarea').fill(text);
    await page.getByTestId('mcp-json-save').click();
    await expect(page.getByTestId('mcp-json-refusal')).toContainText('`bad` has a `cwd`', {
      timeout: 5_000,
    });
    expect(await page.getByTestId('mcp-json-textarea').inputValue()).toBe(text);
  });

  test('a broken buffer is caught locally and can be reverted', async ({ page }) => {
    await page.getByTestId('mcp-json-textarea').fill('{ "mcpServers": ');
    await expect(page.getByTestId('mcp-json-parse-error')).toBeVisible();
    await expect(page.getByTestId('mcp-json-save')).toBeDisabled();
    await page.getByTestId('mcp-json-revert').click();
    await expect(page.getByTestId('mcp-json-parse-error')).toHaveCount(0);
  });
});

test.describe('MCP page — Registry tab', () => {
  let state: MockState;

  test.beforeEach(async ({ page }) => {
    state = { installed: [makeInstalledServer()], statuses: [{ ...STATUS_CONNECTED }] };
    await seedLocalStorage(page);
    await setupMockRpc(page, state);
    await navigateToMcpTab(page);
    await page.getByRole('tab', { name: 'Registry' }).click();
    await expect(page.getByTestId('mcp-registry-browser')).toBeVisible({ timeout: 10_000 });
  });

  function registryRow(page: Page, name: string) {
    return page.getByTestId('mcp-registry-row').filter({ hasText: name });
  }

  test('a hosted server with nothing to fill in is added in one click', async ({ page }) => {
    await page.getByRole('button', { name: 'Add Open Hosted' }).click();
    await expect(
      registryRow(page, 'Open Hosted').getByRole('button', { name: 'Added' })
    ).toBeDisabled({ timeout: 5_000 });
    await expect(page.getByRole('dialog')).toHaveCount(0);
    expect(state.installed.map(s => s.qualified_name)).toContain(OPEN_HOSTED);
    expect(state.statuses.find(s => s.qualified_name === OPEN_HOSTED)?.status).toBe('connected');

    await page.getByRole('tab', { name: 'Servers' }).click();
    const row = installedRow(page, OPEN_HOSTED);
    await expect(row).toBeVisible({ timeout: 5_000 });
    await expect(row).toContainText('https://open-hosted.test/mcp');
  });

  test('adding a server that signs in with OAuth opens the sign-in dialog', async ({ page }) => {
    await page.getByRole('button', { name: 'Add GitHub Tools' }).click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible({ timeout: 5_000 });
    await expect(dialog.getByRole('button', { name: 'Sign in with browser' })).toBeVisible();
    expect(state.installed.map(s => s.qualified_name)).toContain('io.github.test/github-tools');
  });

  test('a declared server is listed as added', async ({ page }) => {
    const row = registryRow(page, 'Memory Server');
    await expect(row).toBeVisible({ timeout: 10_000 });
    await expect(row.getByRole('button', { name: 'Added' })).toBeDisabled();
    await expect(page.getByRole('button', { name: /^Install$/ })).toHaveCount(0);
  });

  test('a server that needs setup writes nothing and falls back to its page', async ({ page }) => {
    await page.getByRole('button', { name: 'Add Tenant Hosted' }).click();
    const row = registryRow(page, 'Tenant Hosted');
    await expect(row.getByTestId('mcp-registry-needs-setup')).toBeVisible({ timeout: 5_000 });
    await expect(
      row.getByRole('button', { name: 'Open the page for Tenant Hosted' })
    ).toBeVisible();
    expect(state.installed).toHaveLength(1);
  });

  test('a refused write shows an inline error with Retry and leaves mcp.json alone', async ({
    page,
  }) => {
    let refused = false;
    await page.route('**/rpc', async (route, request) => {
      const body = JSON.parse(request.postData() || '{}');
      if (body.method === 'openhuman.mcp_clients_config_set' && !refused) {
        refused = true;
        return route.fulfill(rpcError(body.id, 'mcp.json is locked by another save'));
      }
      await route.fallback();
    });
    await page.getByRole('button', { name: 'Add Open Hosted' }).click();
    const error = registryRow(page, 'Open Hosted').getByTestId('mcp-registry-row-error');
    await expect(error).toContainText("Couldn't add this server.", { timeout: 5_000 });
    await expect(error).toContainText('mcp.json is locked by another save');
    expect(state.installed.map(s => s.qualified_name)).toEqual(['io.github.test/memory-server']);

    await error.getByRole('button', { name: 'Try again' }).click();
    await expect(
      registryRow(page, 'Open Hosted').getByRole('button', { name: 'Added' })
    ).toBeDisabled({ timeout: 5_000 });
    expect(state.installed.map(s => s.qualified_name)).toContain(OPEN_HOSTED);
  });

  test("a local server row still opens the server's own page", async ({ page, context }) => {
    // The browser shell is not Tauri here, so `openUrl` falls back to
    // `window.open`; the page it opens is the row's target.
    const popup = context.waitForEvent('page');
    await page.getByRole('button', { name: 'Open the page for Notion Connector' }).click();
    const opened = await popup;
    await expect.poll(() => opened.url()).toBe('https://github.com/test/notion-connector');
    await opened.close();
  });

  test('search with no results shows the empty state', async ({ page }) => {
    await page.route('**/rpc', async (route, request) => {
      const body = JSON.parse(request.postData() || '{}');
      if (
        body.method === 'openhuman.mcp_clients_registry_search' &&
        body.params?.query === 'xyznonexistent999'
      ) {
        return route.fulfill(rpcOk(body.id, { servers: [], page: 1, total_pages: 1 }));
      }
      await route.fallback();
    });
    await page
      .getByTestId('mcp-registry-browser')
      .locator('input[type="search"]')
      .fill('xyznonexistent999');
    await expect(page.getByTestId('mcp-catalog-empty')).toBeVisible({ timeout: 10_000 });
  });
});

test.describe('MCP page — Manage & Uninstall Lifecycle', () => {
  let state: MockState;

  test.beforeEach(async ({ page }) => {
    state = { installed: [makeInstalledServer()], statuses: [{ ...STATUS_CONNECTED }] };
    await seedLocalStorage(page);
    await setupMockRpc(page, state);
    await navigateToMcpTab(page);
  });

  test("a row's name → detail view shows server info and the name", async ({ page }) => {
    await page.getByRole('button', { name: 'Open Memory Server' }).click();
    await expect(page.locator('button:has-text("Back to servers")')).toBeVisible({
      timeout: 5_000,
    });
    await expect(page.locator('text=Memory Server')).toBeVisible();
    await expect(page.locator('text=io.github.test/memory-server')).toBeVisible();
  });

  test('remove flow: row control → confirm → the row is gone', async ({ page }) => {
    await page.getByRole('button', { name: 'Remove Memory Server' }).click();
    const dialog = page.getByTestId('mcp-remove-dialog');
    await expect(dialog).toBeVisible({ timeout: 5_000 });
    await dialog.getByTestId('confirm-dialog-confirm').click();

    await expect(page.getByTestId('mcp-installed-empty')).toBeVisible({ timeout: 10_000 });
    await expect(installedRow(page, 'Memory Server')).toHaveCount(0, { timeout: 5_000 });
  });

  test('uninstall from the detail view also returns to the rows', async ({ page }) => {
    await page.getByRole('button', { name: 'Open Memory Server' }).click();
    await expect(page.locator('button:has-text("Back to servers")')).toBeVisible({
      timeout: 5_000,
    });

    const uninstallBtn = page.locator('button:has-text("Uninstall")');
    await expect(uninstallBtn.first()).toBeVisible({ timeout: 5_000 });
    await uninstallBtn.first().click();

    const confirmBtn = page.locator('button:has-text("Yes")');
    await expect(confirmBtn.first()).toBeVisible({ timeout: 5_000 });
    await confirmBtn.first().click();

    await expect(page.getByTestId('mcp-installed-empty')).toBeVisible({ timeout: 10_000 });
  });

  test('back button from detail returns to the rows', async ({ page }) => {
    await page.getByRole('button', { name: 'Open Memory Server' }).click();
    await expect(page.locator('button:has-text("Back to servers")')).toBeVisible({
      timeout: 5_000,
    });
    await page.locator('button:has-text("Back to servers")').click();
    await expect(page.getByTestId('mcp-servers-section')).toBeVisible({ timeout: 5_000 });
  });
});

test.describe('MCP page — Connect & Tool Execution', () => {
  let state: MockState;

  test.beforeEach(async ({ page }) => {
    // Seed the declared server in `error` status: the detail offers a Connect
    // affordance AND the status poll stays active (error is non-terminal), so
    // the status flips to connected once the modal connects.
    state = {
      installed: [makeInstalledServer()],
      statuses: [
        {
          server_id: 'srv_installed_1',
          qualified_name: 'io.github.test/memory-server',
          display_name: 'Memory Server',
          status: 'error',
          tool_count: 0,
        },
      ],
    };
    await seedLocalStorage(page);
    await setupMockRpc(page, state);
    await navigateToMcpTab(page);
  });

  test('connect a server, then run one of its tools and see the result', async ({ page }) => {
    await page.getByRole('button', { name: 'Open Memory Server' }).click();
    await expect(page.locator('button:has-text("Back to servers")')).toBeVisible({
      timeout: 5_000,
    });

    // Connect via the upfront auth modal (no-auth server → a single Connect
    // button). `exact` avoids the "Connections" sidebar nav button.
    await page.getByRole('button', { name: 'Connect', exact: true }).click();
    const connectDialog = page.getByRole('dialog');
    await expect(connectDialog).toBeVisible({ timeout: 5_000 });
    await connectDialog.getByRole('button', { name: /^Connect$/ }).click();

    // The status poll flips the server to connected → its (collapsed) tool list
    // appears. Expand it, then open the execution playground for a tool.
    const toolsToggle = page.getByRole('button', { name: /tools available/ });
    await expect(toolsToggle).toBeVisible({ timeout: 15_000 });
    await toolsToggle.click();

    const tryButton = page.getByRole('button', {
      name: 'Open execution playground for create_memory',
    });
    await expect(tryButton).toBeVisible({ timeout: 5_000 });
    await tryButton.click();

    const playground = page.getByRole('dialog');
    await expect(playground.getByText('Run create_memory')).toBeVisible({ timeout: 5_000 });
    await playground.getByRole('button', { name: 'Run tool' }).click();
    await expect(page.getByTestId('mcp-playground-result')).toContainText('memory created id=42', {
      timeout: 10_000,
    });
  });
});

test.describe('MCP page — Empty & Edge States', () => {
  test('an empty list routes to the document', async ({ page }) => {
    const state: MockState = { installed: [], statuses: [] };
    await seedLocalStorage(page);
    await setupMockRpc(page, state);
    await navigateToMcpTab(page);

    await expect(page.getByTestId('mcp-installed-empty')).toBeVisible({ timeout: 10_000 });
    await page.getByRole('button', { name: 'Add one in mcp.json' }).click();
    await expect(page.getByTestId('mcp-json-editor')).toBeVisible({ timeout: 10_000 });
  });
});
