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
import { clickButton, textExists, waitForText } from '../helpers/element-helpers';
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

/** Click the action button (Pause | Resume | Remove | …) inside the morning_briefing row. */
async function clickActionForJob(jobName: string, action: string): Promise<boolean> {
  return Boolean(
    await browser.execute(
      (name: string, label: string) => {
        const rows = Array.from(document.querySelectorAll('div'))
          .filter(div => /text-sm font-semibold text-stone-900/.test(div.className))
          .filter(div => (div.textContent ?? '').trim() === name);
        if (rows.length === 0) return false;
        // Walk up to the panel row container (sibling-of-sibling structure in CoreJobList).
        const container = rows[0]?.closest('div.p-4');
        if (!container) return false;
        const buttons = Array.from(container.querySelectorAll<HTMLButtonElement>('button'));
        const btn = buttons.find(b => (b.textContent ?? '').trim() === label);
        if (!btn) return false;
        btn.click();
        return true;
      },
      jobName,
      action
    )
  );
}

/** Poll for the in-row action button label to settle (e.g. "Pause" → "Resume"). */
async function waitForRowActionLabel(
  jobName: string,
  expected: string,
  timeoutMs = 10_000
): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const current = await browser.execute((name: string) => {
      const rows = Array.from(document.querySelectorAll('div'))
        .filter(div => /text-sm font-semibold text-stone-900/.test(div.className))
        .filter(div => (div.textContent ?? '').trim() === name);
      const container = rows[0]?.closest('div.p-4');
      if (!container) return null;
      const labels = Array.from(container.querySelectorAll<HTMLButtonElement>('button')).map(b =>
        (b.textContent ?? '').trim()
      );
      // We care about the toggle button (first one in the row).
      return labels[0] ?? null;
    }, jobName);
    if (current === expected) return true;
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
    const home = await waitForAnyText(
      ['Message OpenHuman', 'Good morning', 'Good afternoon', 'Good evening', 'Upgrade to Premium'],
      15_000
    );
    expect(home).toBeTruthy();
  });

  it('the seeded morning_briefing job appears in the Cron Jobs panel', async () => {
    await openCronJobsPanel();
    // The seed runs in a detached spawn_blocking task — poll for the row.
    const present = await waitForAnyText([MORNING_BRIEFING], 20_000);
    if (!present) {
      stepLog('morning_briefing row never rendered — clicking Refresh and retrying');
      await clickButton('Refresh Cron Jobs');
      await browser.pause(1_500);
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
    await clickButton('Refresh Cron Jobs');
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
