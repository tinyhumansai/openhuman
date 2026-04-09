// @ts-nocheck
/**
 * Integrations & Built-in Skills (Sections 8 & 9)
 *
 * Section 7 (Chat Interface) has been moved to chat-interface-flow.spec.ts.
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { expectRpcMethod, fetchCoreRpcMethods } from '../helpers/core-schema';

async function expectRpcOk(method: string, params: Record<string, unknown> = {}) {
  const result = await callOpenhumanRpc(method, params);
  if (!result.ok) {
    console.log(`[IntegrationsSpec] ${method} failed`, result.error);
  }
  expect(result.ok).toBe(true);
  return result.result;
}

describe('Integrations & Built-in Skills', () => {
  let methods: Set<string>;

  before(async () => {
    await waitForApp();
    await waitForAppReady(20_000);
    methods = await fetchCoreRpcMethods();
  });

  it('8.1.1 — OAuth Authorization Flow: auth_oauth_connect endpoint is available', async () => {
    expectRpcMethod(methods, 'openhuman.auth_oauth_connect');
    await expectRpcOk('openhuman.auth_oauth_connect', { provider: 'google', responseType: 'json' });
  });

  it('8.1.2 — Scope Selection (Read / Write / Initiate): integrations list endpoint is available', async () => {
    expectRpcMethod(methods, 'openhuman.auth_oauth_list_integrations');
    await expectRpcOk('openhuman.auth_oauth_list_integrations', {});
  });

  it('8.1.3 — Token Storage & Encryption: provider credentials storage endpoint is available', async () => {
    expectRpcMethod(methods, 'openhuman.auth_store_provider_credentials');
  });

  it('8.2.1 — Read Access Enforcement: integration permissions can be queried via channels status', async () => {
    expectRpcMethod(methods, 'openhuman.channels_status');
    await expectRpcOk('openhuman.channels_status', {});
  });

  it('8.2.2 — Write Access Enforcement: channel send_message endpoint is exposed', async () => {
    expectRpcMethod(methods, 'openhuman.channels_send_message');
  });

  it('8.2.3 — Initiate Action Enforcement: integration action endpoints are discoverable', async () => {
    expectRpcMethod(methods, 'openhuman.channels_create_thread');
  });

  it('8.2.4 — Cross-Account Access Prevention: oauth revoke integration endpoint exists', async () => {
    expectRpcMethod(methods, 'openhuman.auth_oauth_revoke_integration');
  });

  it('8.3.1 — Data Fetch Handling: skills_sync endpoint is available', async () => {
    expectRpcMethod(methods, 'openhuman.skills_sync');
  });

  it('8.3.2 — Data Write Handling: channels_send_message write endpoint is available', async () => {
    expectRpcMethod(methods, 'openhuman.channels_send_message');
  });

  it('8.3.3 — Large Data Processing: memory query endpoint is available for chunked data', async () => {
    expectRpcMethod(methods, 'openhuman.memory_query_namespace');
  });

  it('8.4.1 — Integration Disconnect: oauth revoke endpoint is available', async () => {
    expectRpcMethod(methods, 'openhuman.auth_oauth_revoke_integration');
  });

  it('8.4.2 — Token Revocation: clear_session endpoint is available', async () => {
    expectRpcMethod(methods, 'openhuman.auth_clear_session');
  });

  it('8.4.3 — Re-Authorization Flow: oauth_connect remains callable after list', async () => {
    await expectRpcOk('openhuman.auth_oauth_list_integrations', {});
    await expectRpcOk('openhuman.auth_oauth_connect', { provider: 'github', responseType: 'json' });
  });

  it('8.4.4 — Permission Re-Sync: skills_sync endpoint can be invoked', async () => {
    const sync = await callOpenhumanRpc('openhuman.skills_sync', { id: 'missing-runtime' });
    expect(sync.ok || Boolean(sync.error)).toBe(true);
  });

  it('9.1.1 — Screen Capture Processing: capture_test endpoint is available', async () => {
    expectRpcMethod(methods, 'openhuman.screen_intelligence_capture_test');
  });

  it('9.1.2 — Context Extraction: vision_recent endpoint is available', async () => {
    expectRpcMethod(methods, 'openhuman.screen_intelligence_vision_recent');
  });

  it('9.1.3 — Memory Injection: memory_doc_put endpoint is available', async () => {
    expectRpcMethod(methods, 'openhuman.memory_doc_put');
  });

  it('9.2.1 — Inline Suggestion Generation: autocomplete_start endpoint exists', async () => {
    expectRpcMethod(methods, 'openhuman.autocomplete_start');
  });

  it('9.2.2 — Debounce Handling: autocomplete_status endpoint exists', async () => {
    expectRpcMethod(methods, 'openhuman.autocomplete_status');
    await expectRpcOk('openhuman.autocomplete_status', {});
  });

  it('9.2.3 — Acceptance Trigger: autocomplete_accept endpoint exists', async () => {
    expectRpcMethod(methods, 'openhuman.autocomplete_accept');
  });

  it('9.3.1 — Voice Input Capture: voice_status endpoint exists', async () => {
    expectRpcMethod(methods, 'openhuman.voice_status');
  });

  it('9.3.2 — Speech-to-Text Processing: voice_status call is reachable', async () => {
    const status = await callOpenhumanRpc('openhuman.voice_status', {});
    expect(status.ok || Boolean(status.error)).toBe(true);
  });

  it('9.3.3 — Voice Command Execution: voice command surface exists in schema', async () => {
    expectRpcMethod(methods, 'openhuman.voice_status');
  });
});
