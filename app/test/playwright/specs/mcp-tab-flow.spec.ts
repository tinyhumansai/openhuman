/**
 * MCP Tab — full lifecycle e2e tests.
 *
 * Covers: browse catalog → search → install (env-key form) → verify in
 * installed list → manage detail view → connect/disconnect → uninstall →
 * verify removal. All RPC calls are mocked via page.route so no running
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
];

function makeInstalledServer(overrides: Partial<typeof INSTALLED_DEFAULT> = {}) {
  return { ...INSTALLED_DEFAULT, ...overrides };
}

const INSTALLED_DEFAULT = {
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

const GITHUB_DETAIL = {
  ...REGISTRY_SERVERS[1],
  connections: [{ type: 'stdio', published: true }],
  required_env_keys: ['GITHUB_TOKEN'],
};

const GITHUB_INSTALLED = {
  server_id: 'srv_github_1',
  qualified_name: 'io.github.test/github-tools',
  display_name: 'GitHub Tools',
  description: 'MCP server for GitHub API integration',
  command_kind: 'node',
  command: 'npx',
  args: ['-y', '@modelcontextprotocol/server-github'],
  env_keys: ['GITHUB_TOKEN'],
  installed_at: 1700000100,
  enabled: true,
};

// ---------------------------------------------------------------------------
// RPC mock layer — mutable state so tests can drive lifecycle transitions
// ---------------------------------------------------------------------------

interface MockState {
  installed: (typeof INSTALLED_DEFAULT)[];
  statuses: (typeof STATUS_CONNECTED)[];
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
                screenIntelligence: { enabled: false },
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
        const installedNames = new Set(state.installed.map(s => s.qualified_name));
        const queryFiltered = query
          ? REGISTRY_SERVERS.filter(
              s =>
                s.display_name.toLowerCase().includes(query) ||
                s.qualified_name.toLowerCase().includes(query)
            )
          : REGISTRY_SERVERS;
        // Exclude servers that are already installed — mirrors real backend behaviour
        const filtered = queryFiltered.filter(s => !installedNames.has(s.qualified_name));
        return route.fulfill(rpcOk(id, { servers: filtered, page: 1, total_pages: 1 }));
      }

      case 'openhuman.mcp_clients_registry_get':
        if (params.qualified_name === GITHUB_DETAIL.qualified_name) {
          return route.fulfill(rpcOk(id, { server: GITHUB_DETAIL }));
        }
        return route.fulfill(rpcError(id, `server not found: ${params.qualified_name}`));

      // ---- Installed servers (mutable) ----
      case 'openhuman.mcp_clients_installed_list':
        return route.fulfill(rpcOk(id, { installed: state.installed }));

      case 'openhuman.mcp_clients_status':
        return route.fulfill(rpcOk(id, { servers: state.statuses }));

      case 'openhuman.mcp_clients_install':
        if (!params.qualified_name) {
          return route.fulfill(rpcError(id, "missing required param 'qualified_name'"));
        }
        state.installed.push(GITHUB_INSTALLED);
        return route.fulfill(rpcOk(id, { server: GITHUB_INSTALLED }));

      case 'openhuman.mcp_clients_connect':
        state.statuses.push({
          server_id: params.server_id,
          qualified_name: 'io.github.test/github-tools',
          display_name: 'GitHub Tools',
          status: 'connected',
          tool_count: 3,
        });
        return route.fulfill(rpcOk(id, { status: 'connected', tools: [] }));

      case 'openhuman.mcp_clients_disconnect':
        state.statuses = state.statuses.filter(s => s.server_id !== params.server_id);
        return route.fulfill(rpcOk(id, { status: 'disconnected' }));

      case 'openhuman.mcp_clients_uninstall':
        state.installed = state.installed.filter(s => s.server_id !== params.server_id);
        state.statuses = state.statuses.filter(s => s.server_id !== params.server_id);
        return route.fulfill(rpcOk(id, { success: true }));

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
  await page.locator('input[type="search"]').waitFor({ state: 'visible', timeout: 10_000 });
  await page.locator('table').waitFor({ state: 'visible', timeout: 10_000 });
}

// ==========================================================================
// Tests
// ==========================================================================

test.describe('MCP Tab — Table View & Filtering', () => {
  let state: MockState;

  test.beforeEach(async ({ page }) => {
    state = { installed: [makeInstalledServer()], statuses: [{ ...STATUS_CONNECTED }] };
    await seedLocalStorage(page);
    await setupMockRpc(page, state);
    await navigateToMcpTab(page);
  });

  test('renders search bar and filter chips', async ({ page }) => {
    await expect(page.locator('input[type="search"]')).toBeVisible();
    await expect(page.getByRole('tab', { name: /^All$/ })).toBeVisible();
    await expect(page.getByRole('tab', { name: /Installed/ })).toBeVisible();
    await expect(page.getByRole('tab', { name: /Registry/ })).toBeVisible();
  });

  test('displays installed servers with status dot and Manage action', async ({ page }) => {
    const row = page.locator('table tbody tr').first();
    await expect(row.locator('td:first-child')).toContainText('Memory Server');
    await expect(row.locator('text=Manage')).toBeVisible();
  });

  test('displays registry servers as clickable rows', async ({ page }) => {
    const registryRow = page.locator('table tbody tr[role="button"]', {
      has: page.locator('text=GitHub Tools'),
    });
    await expect(registryRow).toBeVisible({ timeout: 10_000 });
    await expect(registryRow.locator('text=Install')).toBeVisible();
  });

  test('filter "Installed" hides registry rows', async ({ page }) => {
    await page.getByRole('tab', { name: /Installed/ }).click();
    const rows = page.locator('table tbody tr');
    const count = await rows.count();
    expect(count).toBeGreaterThan(0);
    for (let i = 0; i < count; i++) {
      await expect(rows.nth(i).locator('text=Manage')).toBeVisible();
    }
  });

  test('filter "Registry" hides installed rows', async ({ page }) => {
    await page.getByRole('tab', { name: /Registry/ }).click();
    const rows = page.locator('table tbody tr');
    const count = await rows.count();
    expect(count).toBeGreaterThan(0);
    for (let i = 0; i < count; i++) {
      await expect(rows.nth(i).locator('text=Install')).toBeVisible();
    }
  });

  test('already-installed servers are excluded from registry rows', async ({ page }) => {
    await page.getByRole('tab', { name: /Registry/ }).click();
    const rows = page.locator('table tbody tr');
    const count = await rows.count();
    for (let i = 0; i < count; i++) {
      const text = await rows.nth(i).locator('td:first-child').innerText();
      expect(text).not.toContain('Memory Server');
    }
  });

  test('search filters both installed and registry servers', async ({ page }) => {
    const search = page.locator('input[type="search"]');
    await search.fill('notion');
    // Wait for a Notion row to appear AND for non-matching rows (e.g. "Memory
    // Server") to disappear — the table re-renders asynchronously and a naive
    // count() immediately after the first visible check can race against the
    // previous state still being in the DOM.
    await expect(
      page.locator('table tbody tr', { has: page.locator('td:has-text("Notion")') })
    ).toBeVisible({ timeout: 5_000 });
    await expect(
      page.locator('table tbody tr', { has: page.locator('td:has-text("Memory Server")') })
    ).toHaveCount(0, { timeout: 5_000 });
    // The positive (Notion row present) + negative (Memory Server gone) checks
    // above already prove the filter works. Avoid iterating `td:first-child`
    // per row — the #3480 registry redesign changed the column layout, and the
    // table re-renders async (the old per-row loop raced + assumed name-first).
    await expect(page.locator('table tbody tr')).not.toHaveCount(0);
  });

  test('no Smithery branding visible anywhere', async ({ page }) => {
    // Wait for the table to be fully rendered before scanning body text
    await page.locator('table tbody tr').first().waitFor({ state: 'visible', timeout: 10_000 });
    const bodyText = await page.locator('body').innerText();
    expect(bodyText.toLowerCase()).not.toContain('smithery');
  });
});

test.describe('MCP Tab — Install Lifecycle', () => {
  let state: MockState;

  test.beforeEach(async ({ page }) => {
    state = { installed: [makeInstalledServer()], statuses: [{ ...STATUS_CONNECTED }] };
    await seedLocalStorage(page);
    await setupMockRpc(page, state);
    await navigateToMcpTab(page);
  });

  test('install flow: click row → detail → configure → fill env → submit → appears installed', async ({
    page,
  }) => {
    // 1. Click the GitHub Tools registry row (entire row is clickable)
    const githubRow = page.locator('table tbody tr[role="button"]', {
      has: page.locator('td:first-child:has-text("GitHub Tools")'),
    });
    await expect(githubRow).toBeVisible({ timeout: 10_000 });
    await githubRow.click();

    // 2. Install dialog detail step — shows server info and "Configure & install"
    await expect(page.locator('text=GitHub Tools').first()).toBeVisible({ timeout: 5_000 });
    const configureBtn = page.locator('button:has-text("Configure & install")');
    await expect(configureBtn).toBeVisible({ timeout: 5_000 });
    await configureBtn.click();

    // 3. Configure step — env input appears
    const envInput = page.locator('input[id="env-GITHUB_TOKEN"]');
    await expect(envInput).toBeVisible({ timeout: 5_000 });

    // 4. Fill in the env value
    await envInput.fill('ghp_test_token_123');

    // 5. Click "Install" submit button
    const submitBtn = page.locator('button:has-text("Install")');
    await submitBtn.click();

    // 6. Should navigate to detail view (the installed server detail)
    await expect(page.locator('button:has-text("Go back")')).toBeVisible({ timeout: 10_000 });

    // 7. Go back and verify the server appears in the installed list
    await page.locator('button:has-text("Go back")').click();
    await expect(page.locator('table')).toBeVisible({ timeout: 5_000 });
    const installedGithub = page.locator('table tbody tr', {
      has: page.locator('td:has-text("GitHub Tools")'),
    });
    await expect(installedGithub).toBeVisible({ timeout: 5_000 });
  });

  test('cancel from install dialog returns to table', async ({ page }) => {
    // Click a registry row to open install dialog
    const registryRow = page.locator('table tbody tr[role="button"]', {
      has: page.locator('td:first-child:has-text("GitHub Tools")'),
    });
    await registryRow.click();

    // Cancel button should be visible on detail step
    await expect(page.locator('button:has-text("Cancel")')).toBeVisible({ timeout: 5_000 });
    await page.locator('button:has-text("Cancel")').click();
    await expect(page.locator('table')).toBeVisible({ timeout: 5_000 });
  });
});

test.describe('MCP Tab — Manage & Uninstall Lifecycle', () => {
  let state: MockState;

  test.beforeEach(async ({ page }) => {
    state = { installed: [makeInstalledServer()], statuses: [{ ...STATUS_CONNECTED }] };
    await seedLocalStorage(page);
    await setupMockRpc(page, state);
    await navigateToMcpTab(page);
  });

  test('click installed server row → detail view shows server info', async ({ page }) => {
    const row = page.locator('table tbody tr', {
      has: page.locator('td:first-child:has-text("Memory Server")'),
    });
    await row.click();

    await expect(page.locator('button:has-text("Go back")')).toBeVisible({ timeout: 5_000 });
    await expect(page.locator('text=Memory Server')).toBeVisible();
  });

  test('detail view shows qualified name', async ({ page }) => {
    const row = page.locator('table tbody tr', {
      has: page.locator('td:first-child:has-text("Memory Server")'),
    });
    await row.click();
    await expect(page.locator('button:has-text("Go back")')).toBeVisible({ timeout: 5_000 });
    await expect(page.locator('text=io.github.test/memory-server')).toBeVisible();
  });

  test('uninstall flow: detail → confirm uninstall → returns to table', async ({ page }) => {
    const row = page.locator('table tbody tr', {
      has: page.locator('td:first-child:has-text("Memory Server")'),
    });
    await row.click();
    await expect(page.locator('button:has-text("Go back")')).toBeVisible({ timeout: 5_000 });

    const uninstallBtn = page.locator('button:has-text("Uninstall")');
    await expect(uninstallBtn.first()).toBeVisible({ timeout: 5_000 });
    await uninstallBtn.first().click();

    const confirmBtn = page.locator('button:has-text("Yes")');
    await expect(confirmBtn.first()).toBeVisible({ timeout: 5_000 });
    await confirmBtn.first().click();

    await expect(page.locator('table')).toBeVisible({ timeout: 10_000 });

    await page.getByRole('tab', { name: /Installed/ }).click();
    const removedRow = page.locator('table tbody tr', {
      has: page.locator('td:first-child:has-text("Memory Server")'),
    });
    await expect(removedRow).toHaveCount(0, { timeout: 5_000 });
  });

  test('back button from detail returns to table', async ({ page }) => {
    const row = page.locator('table tbody tr', {
      has: page.locator('td:first-child:has-text("Memory Server")'),
    });
    await row.click();
    await expect(page.locator('button:has-text("Go back")')).toBeVisible({ timeout: 5_000 });
    await page.locator('button:has-text("Go back")').click();
    await expect(page.locator('table')).toBeVisible({ timeout: 5_000 });
  });
});

test.describe('MCP Tab — Empty & Edge States', () => {
  test('empty installed list shows appropriate message', async ({ page }) => {
    const state: MockState = { installed: [], statuses: [] };
    await seedLocalStorage(page);
    await setupMockRpc(page, state);
    await navigateToMcpTab(page);

    await page.getByRole('tab', { name: /Installed/ }).click();
    // Target the empty-state element directly: a broad `text=/no.*servers/i`
    // locator also matches ancestor containers (the root shell wraps the panel),
    // tripping Playwright strict mode.
    await expect(page.getByTestId('mcp-installed-empty')).toBeVisible({ timeout: 10_000 });
  });

  test('search with no results shows no-results message', async ({ page }) => {
    const state: MockState = { installed: [], statuses: [] };
    await seedLocalStorage(page);
    await setupMockRpc(page, state);

    // Registered after setupMockRpc — Playwright routes use LIFO ordering, so
    // this handler runs first and falls through to the base mock for all other
    // methods.
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

    await navigateToMcpTab(page);
    await page.locator('input[type="search"]').fill('xyznonexistent999');

    // Target the catalog empty-state element directly — a broad text regex also
    // matches the root-shell ancestor container and trips strict mode.
    await expect(page.getByTestId('mcp-catalog-empty')).toBeVisible({ timeout: 10_000 });
  });
});
