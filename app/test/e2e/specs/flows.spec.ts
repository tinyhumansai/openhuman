// @ts-nocheck
/**
 * Tinyflows E2E — Workflows create → run → inspect happy path (Phase 6).
 *
 * Drives the real product UI end-to-end (renderer → coreRpcClient → Tauri relay
 * → in-process Rust core, which runs the tinyflows engine locally):
 *
 *   1. Open the Workflows list (`/flows`, `FlowsPage`).
 *   2. Create a workflow from the "New workflow" chooser — the
 *      "Start from scratch" path (`NewWorkflowModal` → `useCreateFlow` →
 *      `openhuman.flows_create`), which persists a minimal single-`manual`-
 *      trigger graph and opens it on the editable canvas (`/flows/:id`).
 *   3. Run it from the canvas (`FlowCanvasPage` Run button →
 *      `openhuman.flows_run`). A trigger-only graph runs to completion in the
 *      local engine with no external calls, so it's deterministic under the
 *      mock backend.
 *   4. Return to the list, open the flow's run history (`FlowRunsDrawer`), pick
 *      the run, and assert the run inspector (`FlowRunInspectorDrawer`) shows a
 *      terminal status (Completed) with at least one recorded step (the trigger
 *      node reconstructs one — see `settle_steps`/`reconstruct_steps` in
 *      `src/openhuman/flows/ops.rs`).
 *
 * Follows the reference structure in `cron-jobs-flow.spec.ts`: ONE Appium
 * session, `resetApp(<unique userId>)` for a fresh-install baseline, then real
 * UI clicks. Everything is targeted via stable `data-testid`s already exposed
 * by the flows components (no raw platform selectors) — the unified Chromium/CDP
 * driver exposes the DOM on all three OSes.
 */
import { waitForApp } from '../helpers/app-helpers';
import { clickTestId, waitForTestId } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-flows';

/** Terminal run-status pill labels (see `flowRuns.status.*` in en.ts). */
const TERMINAL_STATUS_LABELS = ['Completed', 'Failed'];

/** Captured once the scratch flow is created — its `/flows/:id` route id. */
let createdFlowId: string | null = null;

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[FlowsE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[FlowsE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

/** Read the current renderer hash (e.g. `#/flows/<id>`). */
async function currentHash(): Promise<string> {
  return browser.execute(() => window.location.hash);
}

/** Open the Workflows list page and wait for it to render. `/flows` now lands
 *  on the Welcome landing, so jump straight to the functional list view. */
async function openFlowsPage(): Promise<void> {
  await navigateViaHash('/flows?view=main');
  await waitForTestId('flows-page', 15_000);
}

describe('Workflows create → run → inspect (real UI flow)', () => {
  before(async function () {
    // waitForApp() + resetApp() can exceed the default 30s Mocha hook budget.
    this.timeout(120_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('opens the Workflows list from the /flows route', async function () {
    this.timeout(30_000);
    await openFlowsPage();
    // Fresh install → the empty-state "New workflow" affordance is present.
    // The header action button is always rendered regardless of list state.
    await waitForTestId('flows-new-workflow', 10_000);
  });

  it('creates a workflow from scratch and lands on the editable canvas', async function () {
    this.timeout(45_000);

    // Open the Phase 4a chooser and pick "Start from scratch".
    await clickTestId('flows-new-workflow', 10_000);
    await waitForTestId('new-workflow-modal', 10_000);
    await clickTestId('new-workflow-scratch', 10_000);

    // useCreateFlow persists the blank graph via flows_create, then navigates
    // to the new flow's canvas at /flows/:id. Wait for the hash to settle on a
    // concrete id (not the bare /flows list route) and capture it.
    await browser.waitUntil(
      async () => {
        const hash = await currentHash();
        const match = /#\/flows\/([^/]+)$/.exec(hash);
        if (match && match[1] !== 'draft') {
          createdFlowId = match[1];
          return true;
        }
        return false;
      },
      {
        timeout: 20_000,
        interval: 300,
        timeoutMsg: 'canvas route /flows/:id never became active after create',
      }
    );
    stepLog('created flow', { createdFlowId });
    expect(createdFlowId).toBeTruthy();

    // The editable canvas mounted with a runnable flow (Run button present only
    // for a persisted, non-draft flow).
    await waitForTestId('flow-canvas-page', 15_000);
    await waitForTestId('flow-canvas-run', 15_000);
  });

  it('runs the flow from the canvas without error', async function () {
    this.timeout(60_000);

    await clickTestId('flow-canvas-run', 10_000);

    // The run drives the local tinyflows engine. A trigger-only graph settles
    // almost immediately; assert no run-error banner appeared. (The Run button
    // flips to "Running…" and back — we don't assert on that transient state.)
    await browser.pause(2_000);
    const errorBanner = await browser.$('[data-testid="flow-canvas-run-error"]');
    expect(await errorBanner.isExisting()).toBe(false);
  });

  it('inspects the run: terminal status with at least one step', async function () {
    this.timeout(90_000);

    // Back to the Workflows list to reach this flow's run history.
    await openFlowsPage();
    expect(createdFlowId).toBeTruthy();
    const flowId = createdFlowId as string;

    // The created flow's row is present; open its run-history drawer.
    await waitForTestId(`flow-row-${flowId}`, 15_000);
    await clickTestId(`flow-view-runs-${flowId}`, 10_000);
    await waitForTestId('flow-runs-drawer', 10_000);

    // At least one run row exists (the run we just kicked off). Run rows are
    // keyed by run id, which the spec doesn't know ahead of time, so target the
    // first by testid prefix. Poll: the list is a one-shot fetch on open, and
    // the run row is written when the engine settles.
    const runRowSelector = '[data-testid^="flow-run-row-"]';
    await browser.waitUntil(
      async () => {
        const rows = await browser.$$(runRowSelector);
        if (rows.length > 0) return true;
        // Re-open the drawer to re-fetch the (now-settled) run list.
        await clickTestId('flow-runs-close', 5_000).catch(() => undefined);
        await clickTestId(`flow-view-runs-${flowId}`, 5_000).catch(() => undefined);
        await waitForTestId('flow-runs-drawer', 5_000).catch(() => undefined);
        return false;
      },
      {
        timeout: 30_000,
        interval: 1_500,
        timeoutMsg: 'no run rows appeared in the flow runs drawer',
      }
    );

    const firstRunRow = await browser.$(runRowSelector);
    await firstRunRow.click();

    // The run inspector stacks on top; it polls flows_get_run until terminal.
    await waitForTestId('flow-run-inspector-drawer', 15_000);
    await waitForTestId('flow-run-status-pill', 15_000);

    // Poll the status pill until it reads a terminal status (Completed/Failed).
    const pill = await browser.$('[data-testid="flow-run-status-pill"]');
    await browser.waitUntil(
      async () => {
        const text = (await pill.getText().catch(() => '')) ?? '';
        return TERMINAL_STATUS_LABELS.some(label => text.includes(label));
      },
      {
        timeout: 45_000,
        interval: 1_500,
        timeoutMsg: 'run never reached a terminal status in the inspector',
      }
    );
    const statusText = await pill.getText();
    stepLog('run inspector status', { statusText });
    // The trigger-only graph must succeed — assert it specifically completed.
    expect(statusText).toContain('Completed');

    // At least one step is recorded (the trigger node reconstructs one step).
    const steps = await browser.$('[data-testid="flow-run-steps"]');
    await steps.waitForExist({ timeout: 15_000 });
    const firstStep = await browser.$('[data-testid="flow-run-step-0"]');
    expect(await firstStep.isExisting()).toBe(true);
  });
});
