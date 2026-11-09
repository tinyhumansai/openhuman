import { expect } from '@wdio/globals';

import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { clickTestId, waitForTestId } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash, waitForHomePage, waitForRequest } from '../helpers/shared-flows';
import {
  clearRequestLog,
  getRequestLog,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

/**
 * #8081 — opening a Memory tab on a plan that does not include memory signed
 * the user out.
 *
 * The hosted backend answers `memory/scopes` with `403` and
 * `errorCode: MEMORY_PLAN_REQUIRED`. TinyMemory words that "the session expired
 * or the API key was rejected (HTTP 403 …)", and both the core
 * (`is_session_expired_message` → `SessionExpired`) and the renderer
 * (`classifyRpcError` → `core-rpc-auth-expired`) read "session expired" as a
 * sign-out. Here the mock backend refuses every `/memory/*` call that way, the
 * user is on the hosted TinyHumans engine, and each tab must keep the session
 * and show the "not in your plan" prompt instead.
 */

const LOG = '[MemoryPlanRequiredE2E]';
const USER_ID = 'e2e-memory-plan-required';

/** The tabs from the report, each with the alert its view renders on a failed load. */
const TABS = [
  { chip: 'explorer', alert: 'memory-explorer-error' },
  { chip: 'learnings', alert: 'memory-learnings-error' },
  { chip: 'conversations', alert: 'memory-conversations-error' },
] as const;

describe('Memory tabs on a plan without memory (#8081)', () => {
  before(async function () {
    this.timeout(180_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
    const set = await callOpenhumanRpc('openhuman.memory_engine_set', { engine: 'tinyhumans' });
    expect(set.ok).toBe(true);
    setMockBehavior('memoryForceStatus', '403');
    setMockBehavior('memoryForceCode', 'MEMORY_PLAN_REQUIRED');
    console.log(`${LOG} signed in on the tinyhumans engine; /memory/* answers 403 plan-required`);
    await navigateViaHash('/connections?tab=brain&brain=engine');
    await waitForTestId('memory-page', 30_000);
  });

  after(async () => {
    setMockBehavior('memoryForceStatus', '');
    setMockBehavior('memoryForceCode', '');
    await stopMockServer();
  });

  for (const { chip, alert } of TABS) {
    it(`${chip}: shows the plan prompt and keeps the user signed in`, async function () {
      this.timeout(60_000);
      clearRequestLog();
      await clickTestId(`brain-tab-${chip}`);

      // The refusal really came from the hosted backend, so a pass below is
      // not a tab that never asked.
      const refused = await waitForRequest(getRequestLog, 'GET', '/memory/', 20_000);
      expect(refused).toBeDefined();

      const prompt = await waitForTestId(alert, 20_000);
      expect(await prompt.getAttribute('data-kind')).toBe('plan-required');
      const text = await prompt.getText();
      expect(text).toContain("Your current plan doesn't include memory");
      // The engine's raw refusal never reaches the user.
      expect(text).not.toContain('MEMORY_PLAN_REQUIRED');
      await waitForTestId('memory-upgrade', 5_000);

      // Still on the Memory page, not bounced to sign-in.
      await waitForTestId('memory-page', 5_000);
      const hash = await browser.execute(() => window.location.hash);
      expect(hash).toContain('tab=brain');
    });
  }

  it('the session survives the refusals: Home still loads', async function () {
    this.timeout(30_000);
    await navigateViaHash('/home');
    const marker = await waitForHomePage(20_000);
    expect(marker).toBeTruthy();
  });
});
