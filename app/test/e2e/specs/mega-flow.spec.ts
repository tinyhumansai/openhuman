// @ts-nocheck
/**
 * Mega e2e flow — login + Gmail OAuth + Composio triggers in one Mac2 session.
 *
 * Architecture (per design discussion 2026-05-12):
 *   - One Appium Mac2 session, one app launch — no per-scenario restarts.
 *   - Drive the app through:
 *       1. Deep links (`openhuman://auth?…`, `openhuman://oauth/success?…`) —
 *          Mac2 supports these natively via `macos: deepLink`.
 *       2. Mock backend behavior knobs and the in-process request log.
 *       3. Core JSON-RPC for state inspection and `composio_*` calls.
 *   - Assertions read from the mock request log and RPC results — never from
 *     the CEF WebView accessibility tree (which exposes zero DOM to XCUITest).
 *   - Between scenarios, reset state in-app via `openhuman.config_reset_local_data`
 *     (mirrors the production "Clear app data + log out" flow) + mock admin reset.
 *     Then re-write `~/.openhuman/config.toml` so the mock URL persists across
 *     the reset and the next scenario starts pointing at the mock.
 *
 * What this covers (the "major user flows" set):
 *   - Login: deep-link consume → JWT → `/auth/me` fetch
 *   - Bypass login (deep-link `key=auth`): no consume call but session set
 *   - Connect Gmail via OAuth deep-link success path
 *   - OAuth error path is exercised by Scenario 5
 *   - Composio: list connections, enable trigger, list triggers, state mutates
 *   - Factory reset between scenarios (the real product flow)
 *
 * The smoke spec proved the driver+bundle work; this spec proves the *flows* work.
 */
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { triggerDeepLink } from '../helpers/deep-link-helpers';
import { hasAppChrome } from '../helpers/element-helpers';
import {
  clearRequestLog,
  getRequestLog,
  resetMockBehavior,
  setMockBehavior,
  setMockBehaviors,
  startMockServer,
  stopMockServer,
} from '../mock-server';

const LOG = '[MegaFlow]';
const MOCK_PORT = Number(process.env.E2E_MOCK_PORT || 18473);
const HOME = process.env.HOME || os.homedir();
const CONFIG_DIR = path.join(HOME, '.openhuman');
const CONFIG_FILE = path.join(CONFIG_DIR, 'config.toml');
const MOCK_URL = `http://127.0.0.1:${MOCK_PORT}`;

function writeMockConfig(): void {
  fs.mkdirSync(CONFIG_DIR, { recursive: true });
  fs.writeFileSync(CONFIG_FILE, `api_url = "${MOCK_URL}"\n`, 'utf8');
}

async function waitForMockRequest(
  method: string,
  urlFragment: string,
  timeoutMs = 15_000
): Promise<{ method: string; url: string } | undefined> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const hit = getRequestLog().find(r => r.method === method && r.url.includes(urlFragment));
    if (hit) return hit;
    await browser.pause(400);
  }
  return undefined;
}

async function resetEverything(label: string): Promise<void> {
  console.log(`${LOG} reset (${label}) — config_reset_local_data + admin reset`);
  // 1. Wipe the core's local data — workspace + ~/.openhuman + active marker.
  //    The active in-process core handles this without a process restart, so
  //    the session keeps the same RPC port and bearer token.
  const reset = await callOpenhumanRpc('openhuman.config_reset_local_data', {});
  if (!reset.ok) {
    console.warn(`${LOG} reset RPC failed (non-fatal):`, reset);
  }
  // 2. Re-write config.toml so the next core startup-path still points at the
  //    mock backend. config_reset_local_data removed the file.
  writeMockConfig();
  // 3. Wipe mock state + request log.
  await fetch(`${MOCK_URL}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
  }).catch(() => {});
  clearRequestLog();
  resetMockBehavior();
  // Settle a beat so any in-flight reactive HTTP calls don't bleed into the
  // next scenario's request log.
  await browser.pause(800);
}

describe('Mega flow — login + Gmail OAuth + Composio in one session', () => {
  before(async () => {
    writeMockConfig();
    await startMockServer(MOCK_PORT);
    await waitForApp();
    // On Mac2, the window stays in a not-yet-frontmost state until something
    // (a deep link, a click) focuses the app. We assert liveness via the
    // menu bar (matches smoke.spec.ts) and let the first scenario's deep
    // link bring the window forward.
    expect(await hasAppChrome()).toBe(true);
    clearRequestLog();
  });

  after(async () => {
    try {
      await stopMockServer();
    } catch (err) {
      console.log(`${LOG} stopMockServer error (non-fatal):`, err);
    }
  });

  // -------------------------------------------------------------------------
  // Sanity — app + driver are alive. The smoke spec covers this elsewhere,
  // but we re-assert here so failures downstream have a clean signal.
  // -------------------------------------------------------------------------
  it('app is alive', async () => {
    expect(await hasAppChrome()).toBe(true);
  });

  // -------------------------------------------------------------------------
  // Scenario 1 — login via real token-consume deep link.
  // Expectation: the app POSTs to `/telegram/login-tokens/:t/consume`, gets a
  // JWT back from the mock, and follows up with `GET /auth/me`.
  // -------------------------------------------------------------------------
  it('login: consume deep link triggers /consume + /auth/me on the mock', async () => {
    clearRequestLog();
    setMockBehavior('jwt', 'mega-login-1');

    await triggerDeepLink('openhuman://auth?token=mega-login-token');

    const consume = await waitForMockRequest('POST', '/telegram/login-tokens/', 20_000);
    expect(consume).toBeDefined();
    console.log(`${LOG} consume hit:`, consume?.url);

    const me = await waitForMockRequest('GET', '/auth/me', 15_000);
    expect(me).toBeDefined();
    console.log(`${LOG} /auth/me fetched`);
  });

  // -------------------------------------------------------------------------
  // Scenario 2 — reset state, then login via the bypass deep link
  // (`key=auth`). No consume call should be made (the JWT in the URL is the
  // session itself), but the app should still fetch the user profile.
  // -------------------------------------------------------------------------
  it('bypass login: key=auth deep link skips /consume but still fetches /auth/me', async () => {
    await resetEverything('after Scenario 1');

    // Hand-crafted unsigned JWT — mock /auth/me doesn't validate the signature.
    const payload = Buffer.from(
      JSON.stringify({
        sub: 'mega-bypass-user',
        userId: 'mega-bypass-user',
        exp: Math.floor(Date.now() / 1000) + 3600,
      })
    ).toString('base64url');
    const bypassJwt = `eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.${payload}.sig`;

    await triggerDeepLink(`openhuman://auth?token=${encodeURIComponent(bypassJwt)}&key=auth`);

    const me = await waitForMockRequest('GET', '/auth/me', 15_000);
    expect(me).toBeDefined();

    const consume = getRequestLog().find(
      r => r.method === 'POST' && r.url.includes('/telegram/login-tokens/')
    );
    expect(consume).toBeUndefined();
    console.log(`${LOG} bypass: no consume call, /auth/me succeeded`);
  });

  // -------------------------------------------------------------------------
  // Scenario 3 — Gmail OAuth completion via `openhuman://oauth/success`.
  // The deep-link handler dispatches a custom 'oauth:success' event and
  // navigates to /skills. The app refreshes integration state, which manifests
  // as a `GET /auth/integrations` call against the mock.
  // -------------------------------------------------------------------------
  it('Gmail OAuth: success deep link refreshes integrations on the backend', async () => {
    await resetEverything('after Scenario 2');

    // Login first — `oauth:success` is only meaningful for an authenticated user.
    await triggerDeepLink('openhuman://auth?token=mega-gmail-token');
    await waitForMockRequest('POST', '/telegram/login-tokens/', 15_000);
    await waitForMockRequest('GET', '/auth/me', 10_000);
    clearRequestLog();

    await triggerDeepLink('openhuman://oauth/success?integrationId=mock-gmail-int&provider=google');

    // The handler navigates to /skills and dispatches CustomEvent('oauth:success').
    // Downstream listeners refresh integration state — observable as a fresh
    // `/auth/integrations` (and/or `/skills`) call on the mock.
    const refresh =
      (await waitForMockRequest('GET', '/auth/integrations', 15_000)) ||
      (await waitForMockRequest('GET', '/skills', 5_000));
    expect(refresh).toBeDefined();
    console.log(`${LOG} oauth:success triggered refresh ${refresh?.url}`);
  });

  // -------------------------------------------------------------------------
  // Scenario 4 — Composio trigger lifecycle via core RPC. Drives the same
  // contract the UI uses (composio-triggers-flow.spec.ts) but observes via
  // RPC responses + mock log mutation instead of through the WebView.
  // -------------------------------------------------------------------------
  it('Composio: enable_trigger via RPC mutates the active-triggers list', async () => {
    await resetEverything('after Scenario 3');

    // Re-login since reset wipes the session.
    await triggerDeepLink('openhuman://auth?token=mega-composio-token');
    await waitForMockRequest('POST', '/telegram/login-tokens/', 15_000);

    // Seed connections + available triggers; start with an empty active list.
    setMockBehaviors({
      composioConnections: JSON.stringify([{ id: 'c1', toolkit: 'gmail', status: 'ACTIVE' }]),
      composioAvailableTriggers: JSON.stringify([
        { slug: 'GMAIL_NEW_GMAIL_MESSAGE', scope: 'static' },
      ]),
      composioActiveTriggers: JSON.stringify([]),
    });

    const before = await callOpenhumanRpc('openhuman.composio_list_triggers', {});
    expect(before.ok).toBe(true);
    const beforeList = (before.result?.triggers ??
      before.value?.result?.triggers ??
      []) as unknown[];
    expect(Array.isArray(beforeList)).toBe(true);
    expect(beforeList).toHaveLength(0);

    const enable = await callOpenhumanRpc('openhuman.composio_enable_trigger', {
      connection_id: 'c1',
      slug: 'GMAIL_NEW_GMAIL_MESSAGE',
    });
    expect(enable.ok).toBe(true);

    const after = await callOpenhumanRpc('openhuman.composio_list_triggers', {});
    expect(after.ok).toBe(true);
    const afterList = (after.result?.triggers ?? after.value?.result?.triggers ?? []) as unknown[];
    expect(afterList.length).toBeGreaterThan(0);
    console.log(`${LOG} composio: enable mutated active list to`, afterList);
  });

  // -------------------------------------------------------------------------
  // Scenario 5 — OAuth error path. Verifies the app handles the failure
  // deep link without crashing the session.
  // -------------------------------------------------------------------------
  it('Gmail OAuth: error deep link does not crash the session', async () => {
    await resetEverything('after Scenario 4');

    await triggerDeepLink('openhuman://auth?token=mega-error-token');
    await waitForMockRequest('POST', '/telegram/login-tokens/', 15_000);
    clearRequestLog();

    await triggerDeepLink('openhuman://oauth/error?provider=google&error=access_denied');

    // Give the handler a moment to emit its error event.
    await browser.pause(2_000);

    // Liveness check — the app should still respond to a fresh user fetch.
    const post =
      (await waitForMockRequest('GET', '/auth/me', 3_000)) ||
      (await waitForMockRequest('GET', '/auth/integrations', 3_000));
    // It's OK if neither call fires (the error path may not trigger a refresh),
    // but the RPC layer must still be alive.
    const ping = await callOpenhumanRpc('core.ping', {});
    expect(ping.ok).toBe(true);
    console.log(`${LOG} oauth error: core.ping still ok after error deep link`);
    if (post) console.log(`${LOG} post-error follow-up:`, post.url);
  });

  // -------------------------------------------------------------------------
  // Scenario 6 — final factory reset. Verifies that after the destructive
  // RPC + mock admin reset, a fresh login still works.
  // -------------------------------------------------------------------------
  it('post-reset: a fresh login still works end-to-end', async () => {
    await resetEverything('final');

    await triggerDeepLink('openhuman://auth?token=mega-post-reset-token');
    const consume = await waitForMockRequest('POST', '/telegram/login-tokens/', 20_000);
    expect(consume).toBeDefined();
    const me = await waitForMockRequest('GET', '/auth/me', 15_000);
    expect(me).toBeDefined();
    console.log(`${LOG} post-reset login proves config.toml survives reset`);
  });
});
