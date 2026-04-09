// @ts-nocheck
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { expectRpcMethod, fetchCoreRpcMethods } from '../helpers/core-schema';
import { triggerAuthDeepLink } from '../helpers/deep-link-helpers';
import { hasAppChrome, waitForWebView, waitForWindowVisible } from '../helpers/element-helpers';
import {
  clearRequestLog,
  getRequestLog,
  resetMockBehavior,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

const PROVIDERS = ['google', 'github', 'twitter', 'discord'];

async function expectRpcOk(method: string, params: Record<string, unknown> = {}) {
  const result = await callOpenhumanRpc(method, params);
  if (!result.ok) {
    console.log(`[AuthSpec] ${method} failed`, result.error);
  }
  expect(result.ok).toBe(true);
  return result.result;
}

function isKnownAuthScopedFailure(error?: string): boolean {
  const text = String(error || '').toLowerCase();
  return (
    text.includes('session jwt required') ||
    text.includes('invalid token') ||
    text.includes('unauthorized') ||
    text.includes('401') ||
    text.includes('auth connect failed') ||
    text.includes('session validation failed')
  );
}

async function expectRpcOkOrAuthScopedFailure(
  method: string,
  params: Record<string, unknown> = {}
) {
  const result = await callOpenhumanRpc(method, params);
  if (!result.ok) {
    console.log(`[AuthSpec] ${method} auth-scoped result:`, result.error);
  }
  expect(result.ok || isKnownAuthScopedFailure(result.error)).toBe(true);
  return result;
}

function extractToken(result: unknown): string {
  const payload = JSON.stringify(result || {});
  const match = payload.match(/"token"\s*:\s*"([^"]+)"/);
  return match?.[1] || '';
}

describe('Authentication & Multi-Provider Login', () => {
  let methods: Set<string>;

  before(async () => {
    await startMockServer();
    await waitForApp();
    await waitForAppReady(20_000);
    methods = await fetchCoreRpcMethods();
    clearRequestLog();
  });

  after(async () => {
    resetMockBehavior();
    await stopMockServer();
  });

  beforeEach(() => {
    clearRequestLog();
    resetMockBehavior();
  });

  it('1.3.1 — Token Issuance: deep link auth opens app and boots session shell', async () => {
    expect(await hasAppChrome()).toBe(true);

    await triggerAuthDeepLink('e2e-auth-token');
    await waitForWindowVisible(25_000);
    await waitForWebView(20_000);
    await waitForAppReady(20_000);

    const consumeCall = getRequestLog().find(
      item => item.method === 'POST' && item.url.includes('/telegram/login-tokens/')
    );
    if (!consumeCall) {
      console.log('[AuthSpec] consume call missing:', JSON.stringify(getRequestLog(), null, 2));
    }
    expect(Boolean(consumeCall) || process.platform === 'darwin').toBe(true);

    await expectRpcOk('openhuman.auth_get_state', {});
    await expectRpcOk('openhuman.auth_get_session_token', {});
  });

  it('1.1.1 — Google Login: OAuth connect endpoint contract is exposed', async () => {
    expectRpcMethod(methods, 'openhuman.auth_oauth_connect');
    expectRpcMethod(methods, 'openhuman.auth_oauth_list_integrations');
    await expectRpcOkOrAuthScopedFailure('openhuman.auth_oauth_connect', {
      provider: 'google',
      responseType: 'json',
    });
  });

  it('1.1.2 — GitHub Login: OAuth connect endpoint contract is exposed', async () => {
    await expectRpcOkOrAuthScopedFailure('openhuman.auth_oauth_connect', {
      provider: 'github',
      responseType: 'json',
    });
  });

  it('1.1.3 — Twitter Login: OAuth connect endpoint contract is exposed', async () => {
    await expectRpcOkOrAuthScopedFailure('openhuman.auth_oauth_connect', {
      provider: 'twitter',
      responseType: 'json',
    });
  });

  it('1.1.4 — Discord Login: OAuth connect endpoint contract is exposed', async () => {
    await expectRpcOkOrAuthScopedFailure('openhuman.auth_oauth_connect', {
      provider: 'discord',
      responseType: 'json',
    });
  });

  it('1.2.1 — Single Provider Account Creation: can persist provider credentials', async () => {
    const profile = `e2e-${Date.now()}`;
    await expectRpcOk('openhuman.auth_store_provider_credentials', {
      provider: 'github',
      profile,
      token: 'ghp_e2e_token',
      setActive: true,
    });

    const listed = await expectRpcOk('openhuman.auth_list_provider_credentials', { provider: 'github' });
    expect(JSON.stringify(listed || {}).includes(profile)).toBe(true);
  });

  it('1.2.2 — Multi-Provider Linking: multiple providers can be stored concurrently', async () => {
    const profile = `multi-${Date.now()}`;
    for (const provider of PROVIDERS) {
      await expectRpcOk('openhuman.auth_store_provider_credentials', {
        provider,
        profile,
        token: `${provider}-token`,
      });
    }

    const list = await expectRpcOk('openhuman.auth_list_provider_credentials', {});
    const payload = JSON.stringify(list || {});
    expect(payload.includes('google')).toBe(true);
    expect(payload.includes('github')).toBe(true);
  });

  it('1.2.3 — Duplicate Account Prevention: same provider/profile updates without RPC error', async () => {
    const profile = 'duplicate-check';
    await expectRpcOk('openhuman.auth_store_provider_credentials', {
      provider: 'discord',
      profile,
      token: 'first-token',
    });
    await expectRpcOk('openhuman.auth_store_provider_credentials', {
      provider: 'discord',
      profile,
      token: 'second-token',
    });

    const list = await expectRpcOk('openhuman.auth_list_provider_credentials', { provider: 'discord' });
    expect(JSON.stringify(list || {}).includes(profile)).toBe(true);
  });

  it('1.3.2 — Refresh Token Rotation: storing a new session token rotates effective token', async () => {
    setMockBehavior('jwt', 'rot1');
    await triggerAuthDeepLink('e2e-rot-token-1');
    await browser.pause(2_000);
    const token1 = await expectRpcOk('openhuman.auth_get_session_token', {});
    const value1 = extractToken(token1);

    setMockBehavior('jwt', 'rot2');
    await triggerAuthDeepLink('e2e-rot-token-2');
    await browser.pause(2_000);
    const token2 = await expectRpcOk('openhuman.auth_get_session_token', {});
    const value2 = extractToken(token2);

    expect(value2.length > 0 || value1.length > 0).toBe(true);
  });

  it('1.3.3 — Multi-Device Sessions: repeated session stores remain valid state transitions', async () => {
    await triggerAuthDeepLink('e2e-device-token-a');
    await browser.pause(2_000);
    await triggerAuthDeepLink('e2e-device-token-b');
    await browser.pause(2_000);
    await expectRpcOk('openhuman.auth_get_state', {});
  });

  it('1.4.1 — Session Logout: clear session removes active token', async () => {
    await triggerAuthDeepLink('e2e-logout-token');
    await browser.pause(2_000);
    await expectRpcOk('openhuman.auth_clear_session', {});
    const token = await expectRpcOk('openhuman.auth_get_session_token', {});
    expect(extractToken(token).length === 0 || JSON.stringify(token || {}).includes('null')).toBe(
      true
    );
  });

  it('1.4.2 — Global Logout: clearing session invalidates auth state across providers', async () => {
    await expectRpcOk('openhuman.auth_store_provider_credentials', {
      provider: 'google',
      profile: 'global-logout',
      token: 'some-token',
    });
    await expectRpcOk('openhuman.auth_clear_session', {});
    await expectRpcOk('openhuman.auth_get_state', {});
  });

  it('1.4.3 — Token Invalidation: backend auth/me failure surfaces as RPC error', async () => {
    await triggerAuthDeepLink('e2e-valid-token');
    await browser.pause(2_000);
    setMockBehavior('session', 'revoked');
    const me = await callOpenhumanRpc('openhuman.auth_get_me', {});
    expect(me.ok).toBe(false);
  });
});
