import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc } from '../helpers/core-rpc';

interface ServerStatus {
  running?: boolean;
  url?: string;
}

function unwrapStatus(raw: unknown): ServerStatus {
  const root = raw as { result?: ServerStatus } & ServerStatus;
  return root.result ?? root;
}

interface AgentDef {
  id?: string;
  tools?: unknown;
}

interface ListDefinitionsResult {
  definitions?: AgentDef[];
}

test.describe('System tools - Browser (open URL + automation registry)', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, 'pw-tool-browser-' + testSlug, '/home');
  });

  test('agent runtime is reachable and tools_agent is registered', async () => {
    const status = unwrapStatus(await callCoreRpc<unknown>('openhuman.agent_server_status', {}));
    expect(status.running).toBe(true);

    const list = await callCoreRpc<ListDefinitionsResult>('openhuman.agent_list_definitions', {});
    const defs = list.definitions ?? [];
    const toolsAgent = defs.find(def => def?.id === 'tools_agent');
    expect(toolsAgent).toBeDefined();
    expect(toolsAgent?.tools).toBeDefined();
  });

  test('browser-bearing agent definitions are exposed in the live registry', async () => {
    const list = await callCoreRpc<ListDefinitionsResult>('openhuman.agent_list_definitions', {});
    const defs = list.definitions ?? [];
    const browserBearing = defs.filter(def =>
      ['tools_agent', 'integrations_agent', 'researcher', 'planner'].includes(def?.id ?? '')
    );
    expect(browserBearing.length).toBeGreaterThan(0);
  });

  test.skip('future chat tool_calls drive browser_open end-to-end via deterministic mock LLM', async () => {});
});
