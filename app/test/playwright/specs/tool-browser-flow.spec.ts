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
  direct_tool_names?: string[];
}

interface ListDefinitionsResult {
  definitions?: AgentDef[];
}

test.describe('System tools - Browser (open URL + automation registry)', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, 'pw-tool-browser-' + testSlug, '/home');
  });

  test('agent runtime is reachable and the orchestrator can discover tools', async () => {
    const status = unwrapStatus(await callCoreRpc<unknown>('openhuman.agent_server_status', {}));
    expect(status.running).toBe(true);

    const list = await callCoreRpc<ListDefinitionsResult>('openhuman.agent_list_definitions', {});
    const defs = list.definitions ?? [];
    const orchestrator = defs.find(def => def?.id === 'orchestrator');
    expect(orchestrator).toBeDefined();
    // Browser tools are reached through on-demand discovery, not a
    // browser-bearing specialist (`tools_agent` / `integrations_agent` /
    // `researcher` are retired).
    expect(orchestrator?.direct_tool_names ?? []).toContain('tool_search');
    for (const retired of ['tools_agent', 'integrations_agent', 'researcher']) {
      expect(defs.find(def => def?.id === retired)).toBeUndefined();
    }
  });

});
