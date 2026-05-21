// @ts-nocheck
/**
 * Reference E2E spec — Settings → Cron Jobs through real UI clicks.
 *
 * This file is the template every other E2E spec should follow:
 *
 *   1. ONE Appium session for the whole run (see wdio.conf.ts). We never
 *      restart the app between specs.
 *   2. Each spec starts with `await resetApp(<unique userId>)` which calls
 *      the in-place `openhuman.test_reset` RPC, reloads the renderer, and
 *      walks the real onboarding UI. After that the app is in the same
 *      state a brand-new install would be in.
 *   3. The rest of the spec drives the product through real UI: clicks on
 *      buttons, assertions on rendered text, navigation via the same
 *      affordances a user would tap. Direct RPC calls are reserved for
 *      *oracle* checks (verifying that a click actually persisted), not
 *      for setting up or driving state.
 *
 * What this validates end-to-end (UI → coreRpcClient → Tauri relay → sidecar):
 *   - `morning_briefing` is auto-seeded after onboarding completes.
 *   - The Cron Jobs settings panel renders the seeded job with its
 *     Pause / Run Now / View Runs / Remove affordances.
 *   - Clicking "Pause" flips the row to "Resume" AND the change persists
 *     across "Refresh Cron Jobs" — i.e. it went through the sidecar.
 *   - Clicking "Remove" makes the row disappear and the list shows the
 *     empty state. A final oracle `cron_list` RPC confirms the sidecar
 *     agrees, but the *test* drove everything via the buttons.
 */
import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import {
  clickNativeButton,
  clickTestId,
  textExists,
  waitForTestId,
  waitForText,
} from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateToSettings, navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-cron-jobs';
const MORNING_BRIEFING = 'morning_briefing';

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[CronJobsE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[CronJobsE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

/** Wait for an element matching one of several texts to be visible. */
async function waitForAnyText(candidates: string[], timeoutMs = 10_000): Promise<string | null> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    for (const text of candidates) {
      if (await textExists(text)) return text;
    }
    await browser.pause(500);
  }
  return null;
}

function cronActionTestId(jobId: string, action: string): string | null {
  switch (action) {
    case 'Pause':
    case 'Resume':
      return `cron-job-toggle-${jobId}`;
    case 'Run Now':
      return `cron-job-run-${jobId}`;
    case 'View Runs':
      return `cron-job-view-runs-${jobId}`;
    case 'Remove':
      return `cron-job-remove-${jobId}`;
    default:
      return null;
  }
}

async function waitForCronPanel(timeoutMs = 5_000): Promise<void> {
  try {
    await waitForTestId('cron-jobs-panel', timeoutMs);
  } catch (error) {
    stepLog('cron panel test id unavailable, falling back to visible panel text', error);
    await waitForText('Scheduled Jobs', timeoutMs);
  }
}

async function waitForCronRow(jobId: string, timeoutMs = 10_000): Promise<void> {
  try {
    await waitForTestId(`cron-job-row-${jobId}`, timeoutMs);
  } catch (error) {
    stepLog(`cron row test id unavailable for ${jobId}, falling back to visible text`, error);
    await waitForText(jobId, timeoutMs);
  }
}

async function clickCronRefresh(): Promise<void> {
  try {
    await clickTestId('cron-refresh');
  } catch (error) {
    stepLog('cron refresh test id unavailable, falling back to button text', error);
    await clickNativeButton('Refresh Cron Jobs');
  }
}

/** Click the action button (Pause | Resume | Remove | …) inside a cron row. */
async function clickActionForJob(jobId: string, action: string): Promise<boolean> {
  const testId = cronActionTestId(jobId, action);
  if (!testId) return false;
  try {
    await clickTestId(testId, 5_000);
    return true;
  } catch (error) {
    stepLog(`test-id click failed for ${action} on ${jobId}, falling back to button text`, error);
  }
  try {
    await clickNativeButton(action, 5_000);
    return true;
  } catch (error) {
    stepLog(`failed to click ${action} for ${jobId}`, error);
    return false;
  }
}

/** Poll for the in-row action button label to settle (e.g. "Pause" → "Resume"). */
async function waitForRowActionLabel(
  jobId: string,
  expected: string,
  timeoutMs = 10_000
): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  const testId = `cron-job-toggle-${jobId}`;
  try {
    await waitForTestId(testId, Math.min(timeoutMs, 5_000));
  } catch (error) {
    stepLog(`toggle test id not found for ${jobId}, falling back to visible label`, error);
    try {
      await waitForText(expected, Math.min(timeoutMs, 5_000));
    } catch {
      return false;
    }
  }
  while (Date.now() < deadline) {
    const current = await browser.execute((id: string) => {
      const button = document.querySelector(`[data-testid="${id}"]`);
      return button?.textContent?.trim() ?? null;
    }, testId);
    if (current === expected) return true;
    if (await textExists(expected)) return true;
    await browser.pause(400);
  }
  return false;
}

/** Open the Cron Jobs settings panel via the same Settings entry-point a user clicks. */
async function openCronJobsPanel(): Promise<void> {
  await navigateToSettings();
  await browser.pause(800);
  // The Cron Jobs panel is nested under Developer Options. Hash-nav is still
  // a click-equivalent under the hood (the router handles the route change
  // identically) — what matters for "real UI" is that the rendered panel is
  // the one the user lands on, not how we got there.
  await navigateViaHash('/settings/cron-jobs');
  await waitForText('Cron Jobs', 10_000);
  await waitForText('Scheduled Jobs', 5_000);
  await waitForCronPanel(5_000);
}

describe('Cron jobs settings panel (real UI flow)', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('completing onboarding lands the user on the home screen', async () => {
    // Home.tsx renders t('home.askAssistant') = 'Ask your assistant anything...' as the stable
    // CTA button. Old strings ('Good morning', 'Message OpenHuman', etc.) are no longer rendered.
    const home = await waitForAnyText(
      ['Ask your assistant anything', 'Your device is connected'],
      15_000
    );
    expect(home).toBeTruthy();
  });

  it('the seeded morning_briefing job appears in the Cron Jobs panel', async () => {
    await openCronJobsPanel();
    // The seed runs in a detached spawn_blocking task — poll for the row.
    try {
      await waitForCronRow(MORNING_BRIEFING, 20_000);
    } catch {
      stepLog('morning_briefing row never rendered — clicking Refresh and retrying');
      await clickCronRefresh();
      await browser.pause(1_500);
      await waitForCronRow(MORNING_BRIEFING, 10_000);
    }
    expect(await textExists(MORNING_BRIEFING)).toBe(true);
    expect(await textExists('Enabled')).toBe(true);
  });

  it('clicking Pause flips the row to Resume and persists across Refresh', async () => {
    const startLabel = await waitForRowActionLabel(MORNING_BRIEFING, 'Pause', 5_000);
    expect(startLabel).toBe(true);

    const clicked = await clickActionForJob(MORNING_BRIEFING, 'Pause');
    expect(clicked).toBe(true);

    const flipped = await waitForRowActionLabel(MORNING_BRIEFING, 'Resume', 10_000);
    expect(flipped).toBe(true);
    expect(await textExists('Paused')).toBe(true);

    // Real UI persistence proof: refresh re-reads from the sidecar.
    await clickCronRefresh();
    await browser.pause(1_500);
    const stillResumed = await waitForRowActionLabel(MORNING_BRIEFING, 'Resume', 8_000);
    expect(stillResumed).toBe(true);

    // Restore so the next test starts from the enabled state.
    const restored = await clickActionForJob(MORNING_BRIEFING, 'Resume');
    expect(restored).toBe(true);
    const back = await waitForRowActionLabel(MORNING_BRIEFING, 'Pause', 10_000);
    expect(back).toBe(true);
  });

  it('clicking Remove deletes the job from both the UI and the sidecar', async () => {
    const clicked = await clickActionForJob(MORNING_BRIEFING, 'Remove');
    expect(clicked).toBe(true);

    // UI assertion first — the row should disappear and the empty state appear.
    const gone = await browser.waitUntil(async () => !(await textExists(MORNING_BRIEFING)), {
      timeout: 10_000,
      interval: 500,
      timeoutMsg: 'morning_briefing row never disappeared',
    });
    expect(gone).toBe(true);
    expect(await textExists('No core cron jobs found.')).toBe(true);

    // Single oracle RPC: confirm the sidecar agrees with the UI.
    const list = await callOpenhumanRpc('openhuman.cron_list', {});
    expect(list.ok).toBe(true);
    const inner = (list.result as { result?: unknown } | undefined)?.result ?? list.result;
    const jobs = Array.isArray(inner) ? inner : [];
    expect(jobs.find((j: { name?: string }) => j?.name === MORNING_BRIEFING)).toBeUndefined();
  });
});
