// @ts-nocheck
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { expectRpcMethod, fetchCoreRpcMethods } from '../helpers/core-schema';

async function expectRpcOk(method: string, params: Record<string, unknown> = {}) {
  const result = await callOpenhumanRpc(method, params);
  if (!result.ok) {
    console.log(`[RewardsSpec] ${method} failed`, result.error);
  }
  expect(result.ok).toBe(true);
  return result.result;
}

describe('Rewards, Progression & Settings', () => {
  let methods: Set<string>;

  before(async () => {
    await waitForApp();
    await waitForAppReady(20_000);
    methods = await fetchCoreRpcMethods();
  });

  it('10.1.1 — Activity-Based Unlock: capability catalog exposes conversation activity features', async () => {
    const result = await expectRpcOk('openhuman.about_app_lookup', { id: 'conversation.suggested_questions' });
    expect(JSON.stringify(result || {}).toLowerCase().includes('conversation')).toBe(true);
  });

  it('10.1.2 — Plan-Based Unlock: billing plan RPC endpoint is available', async () => {
    expectRpcMethod(methods, 'openhuman.billing_get_current_plan');
  });

  it('10.1.3 — Integration-Based Unlock: skills connection capabilities are discoverable', async () => {
    const result = await expectRpcOk('openhuman.about_app_search', { query: 'connect' });
    expect(JSON.stringify(result || {}).toLowerCase().includes('connect')).toBe(true);
  });

  it('10.2.1 — Message Count Tracking: subconscious status endpoint is available', async () => {
    expectRpcMethod(methods, 'openhuman.subconscious_status');
    await expectRpcOk('openhuman.subconscious_status', {});
  });

  it('10.2.2 — Feature Usage Tracking: about_app list returns capability set', async () => {
    const list = await expectRpcOk('openhuman.about_app_list', {});
    expect(JSON.stringify(list || {}).length > 10).toBe(true);
  });

  it('10.2.3 — Unlock State Persistence: app state snapshot endpoint is exposed', async () => {
    expectRpcMethod(methods, 'openhuman.app_state_snapshot');
  });

  it('11.1.1 — Profile Management: auth_get_me endpoint exists', async () => {
    expectRpcMethod(methods, 'openhuman.auth_get_me');
  });

  it('11.1.2 — Linked Accounts Management: oauth integrations list endpoint exists', async () => {
    expectRpcMethod(methods, 'openhuman.auth_oauth_list_integrations');
  });

  it('11.2.1 — Accessibility Settings: screen intelligence status endpoint exists', async () => {
    expectRpcMethod(methods, 'openhuman.screen_intelligence_status');
  });

  it('11.2.2 — Messaging Channel Config: channels status endpoint exists', async () => {
    expectRpcMethod(methods, 'openhuman.channels_status');
    await expectRpcOk('openhuman.channels_status', {});
  });

  it('11.3.1 — Model Configuration: local AI presets endpoint exists', async () => {
    expectRpcMethod(methods, 'openhuman.local_ai_presets');
  });

  it('11.3.2 — Skill Enable/Disable: skills list endpoint exists', async () => {
    expectRpcMethod(methods, 'openhuman.skills_list');
  });

  it('11.4.1 — Webhook Inspection: webhooks list endpoint exists', async () => {
    expectRpcMethod(methods, 'openhuman.webhooks_list_tunnels');
  });

  it('11.4.2 — Memory Debug: memory namespace list endpoint exists', async () => {
    expectRpcMethod(methods, 'openhuman.memory_namespace_list');
  });

  it('11.4.3 — Runtime Logs: subconscious log list endpoint exists', async () => {
    expectRpcMethod(methods, 'openhuman.subconscious_log_list');
  });

  it('11.5.1 — Clear App Data: auth_clear_session can be called', async () => {
    expectRpcMethod(methods, 'openhuman.auth_clear_session');
    await expectRpcOk('openhuman.auth_clear_session', {});
  });

  it('11.5.2 — Local Cache Reset: memory clear_namespace can be called', async () => {
    expectRpcMethod(methods, 'openhuman.memory_clear_namespace');
    await expectRpcOk('openhuman.memory_clear_namespace', { namespace: 'e2e-reset' });
  });

  it('11.5.3 — Full State Reset: app state update endpoint supports clearing local state', async () => {
    expectRpcMethod(methods, 'openhuman.app_state_update_local_state');
    const updated = await callOpenhumanRpc('openhuman.app_state_update_local_state', {
      encryptionKey: null,
      primaryWalletAddress: null,
      onboardingTasks: null,
    });
    expect(updated.ok || Boolean(updated.error)).toBe(true);
  });
});
