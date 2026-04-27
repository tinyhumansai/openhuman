import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import { waitForWebView, waitForWindowVisible } from '../helpers/element-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import { completeOnboardingIfVisible } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

/**
 * Memory subsystem round-trip spec (features 8.1.1 store / 8.1.2 recall /
 * 8.1.3 forget).
 *
 * Goal: prove that the JSON-RPC memory API is wired end-to-end through the
 * Tauri shell and core sidecar — store a fact, recall it via search, then
 * forget it and confirm the recall path no longer returns it.
 *
 * Driven via `callOpenhumanRpc` rather than UI navigation: the user-visible
 * surface (Intelligence dashboard) is asserted in `insights-dashboard.spec.ts`.
 * Keeping this spec narrow to the RPC contract makes regressions in the
 * memory sidecar easy to bisect.
 *
 * Failure path: forget-then-recall must return zero hits — that's the
 * 8.1.3 edge assertion required by docs/TESTING-STRATEGY.md.
 */
function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[MemoryRoundTripE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[MemoryRoundTripE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

const TEST_NAMESPACE = 'e2e-memory-roundtrip-773';
const TEST_KEY = 'roundtrip-canary-key';
const TEST_TITLE = 'Memory roundtrip canary';
const TEST_CONTENT = 'OpenHuman memory roundtrip canary fact #773';

describe('Memory subsystem round-trip', () => {
  before(async function beforeSuite() {
    if (!supportsExecuteScript()) {
      stepLog('Skipping suite on Mac2 — core-rpc helper is browser.execute-bound');
      this.skip();
    }

    stepLog('starting mock server');
    await startMockServer();
    stepLog('waiting for app');
    await waitForApp();
    stepLog('triggering auth bypass deep link');
    await triggerAuthDeepLinkBypass('e2e-memory-roundtrip');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[MemoryRoundTripE2E]');

    // Memory subsystem must be initialised before doc_put / recall.
    stepLog('initialising memory subsystem');
    const init = await callOpenhumanRpc('openhuman.memory_init', { jwt_token: '' });
    stepLog('memory_init response', init);
    expect(init.ok).toBe(true);

    // Make sure the namespace starts empty so the recall assertion in test 1
    // is unambiguous if a previous run left state behind.
    stepLog('clearing namespace pre-suite');
    await callOpenhumanRpc('openhuman.memory_clear_namespace', { namespace: TEST_NAMESPACE });
  });

  after(async () => {
    stepLog('stopping mock server');
    await stopMockServer();
  });

  it('stores a document via memory_doc_put and finds it via recall_memories', async () => {
    stepLog('storing memory');
    const storeResult = await callOpenhumanRpc('openhuman.memory_doc_put', {
      namespace: TEST_NAMESPACE,
      key: TEST_KEY,
      title: TEST_TITLE,
      content: TEST_CONTENT,
    });
    stepLog('store response', storeResult);
    expect(storeResult.ok).toBe(true);

    stepLog('recalling memory');
    const recallResult = await callOpenhumanRpc('openhuman.memory_recall_memories', {
      namespace: TEST_NAMESPACE,
      limit: 10,
    });
    stepLog('recall response', recallResult);
    expect(recallResult.ok).toBe(true);
    const recalled = JSON.stringify(recallResult.result ?? {});
    expect(recalled.includes(TEST_KEY) || recalled.includes(TEST_CONTENT)).toBe(true);
  });

  it('clears a namespace and recall returns no canary content (edge case)', async () => {
    stepLog('clearing namespace');
    const forgetResult = await callOpenhumanRpc('openhuman.memory_clear_namespace', {
      namespace: TEST_NAMESPACE,
    });
    stepLog('clear response', forgetResult);
    expect(forgetResult.ok).toBe(true);

    stepLog('recalling after clear — must miss');
    const recallAfterForget = await callOpenhumanRpc('openhuman.memory_recall_memories', {
      namespace: TEST_NAMESPACE,
      limit: 10,
    });
    stepLog('post-clear recall response', recallAfterForget);
    expect(recallAfterForget.ok).toBe(true);
    const recalled = JSON.stringify(recallAfterForget.result ?? {});
    expect(recalled.includes(TEST_KEY)).toBe(false);
    expect(recalled.includes(TEST_CONTENT)).toBe(false);
  });
});
