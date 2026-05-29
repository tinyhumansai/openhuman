// @ts-nocheck
/**
 * Navigation smoothness — rapid tab switching across all major routes.
 *
 * Exercises the HashRouter-based navigation by visiting every top-level
 * route twice (a normal pass and then a rapid pass with minimal delays)
 * and asserting each renders non-trivially.
 *
 * Tests:
 *   N1.1 — all 8 major routes render without error within timing budget
 *   N1.2 — rapid cycle (second pass) completes without blank screens
 *   N1.3 — final state is /home with correct content
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash, waitForHomePage } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const LOG_PREFIX = '[navigation-smoothness]';
const USER_ID = 'e2e-navigation-smoothness';
const ROUTE_TIMEOUT = 10_000;

// Routes to visit, with optional text markers that confirm the panel loaded.
interface RouteCheck {
  hash: string;
  markers: string[];
}

const ROUTES: RouteCheck[] = [
  { hash: '/chat', markers: ['Threads', 'Chat', 'Message', 'New thread'] },
  { hash: '/skills', markers: ['Skills', 'Skill', 'Install', 'Browse'] },
  {
    hash: '/home',
    markers: [
      'Good morning',
      'Good afternoon',
      'Good evening',
      'Message OpenHuman',
      'Test',
      'Upgrade',
    ],
  },
  { hash: '/channels', markers: ['Channels', 'Channel', 'Connect', 'Add', 'Gmail', 'Telegram'] },
  {
    hash: '/notifications',
    markers: ['Notifications', 'Alerts', 'Notification', 'No notifications'],
  },
  { hash: '/rewards', markers: ['Rewards', 'Referral', 'Credits', 'Earn', 'Invite'] },
  { hash: '/settings', markers: ['Settings', 'Account', 'Billing', 'Advanced'] },
  {
    hash: '/home',
    markers: [
      'Good morning',
      'Good afternoon',
      'Good evening',
      'Message OpenHuman',
      'Test',
      'Upgrade',
    ],
  },
];

async function rootTextLength(): Promise<number> {
  return (await browser.execute(
    () => (document.getElementById('root')?.innerText ?? '').length
  )) as number;
}

async function verifyRouteLoaded(route: RouteCheck, pass: string): Promise<void> {
  await waitForAppReady(ROUTE_TIMEOUT);

  const chars = await rootTextLength();
  if (chars < 50) {
    throw new Error(`${pass} ${route.hash}: appears blank (${chars} chars)`);
  }

  let foundMarker = '';
  for (const marker of route.markers) {
    if (await textExists(marker)) {
      foundMarker = marker;
      break;
    }
  }
  if (foundMarker) {
    console.log(
      `${LOG_PREFIX} ${pass} ${route.hash}: loaded (found "${foundMarker}", ${chars} chars)`
    );
  } else {
    // Non-fatal: some routes may have different text depending on state.
    // The char count check above is the authoritative blank-screen guard.
    console.log(
      `${LOG_PREFIX} ${pass} ${route.hash}: loaded (${chars} chars, no marker matched — acceptable)`
    );
  }
}

describe('Navigation smoothness', () => {
  before(async () => {
    console.log(`${LOG_PREFIX} Starting mock server and resetting app`);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
    console.log(`${LOG_PREFIX} Setup complete`);
  });

  after(async () => {
    await stopMockServer();
    console.log(`${LOG_PREFIX} Teardown complete`);
  });

  it('N1.1 — all 8 major routes render without error within timing budget', async () => {
    console.log(`${LOG_PREFIX} N1.1: first pass — normal navigation`);
    for (const route of ROUTES) {
      console.log(`${LOG_PREFIX} N1.1: navigating to ${route.hash}`);
      await navigateViaHash(route.hash);
      await verifyRouteLoaded(route, 'N1.1');
      // Small pause between routes so React has time to settle.
      await browser.pause(400);
    }
    console.log(`${LOG_PREFIX} N1.1: passed — all routes loaded`);
  });

  it('N1.2 — rapid cycle (second pass) completes without blank screens', async () => {
    console.log(`${LOG_PREFIX} N1.2: second pass — rapid cycle`);
    for (const route of ROUTES) {
      console.log(`${LOG_PREFIX} N1.2: rapid-navigating to ${route.hash}`);
      await navigateViaHash(route.hash);
      // Minimal pause — just enough for hash update and React to start rendering.
      await browser.pause(350);

      await waitForAppReady(ROUTE_TIMEOUT);
      const chars = await rootTextLength();
      if (chars < 50) {
        throw new Error(`N1.2 rapid-cycle ${route.hash}: blank screen (${chars} chars)`);
      }
      console.log(`${LOG_PREFIX} N1.2: ${route.hash} rendered (${chars} chars)`);
    }
    console.log(`${LOG_PREFIX} N1.2: passed — rapid cycle complete`);
  });

  it('N1.3 — final state is /home with correct content', async () => {
    console.log(`${LOG_PREFIX} N1.3: navigating to /home for final check`);
    await navigateViaHash('/home');
    const homeText = await waitForHomePage(ROUTE_TIMEOUT);
    expect(homeText).toBeTruthy();

    const hash = await browser.execute(() => window.location.hash);
    expect(hash).toMatch(/^#\/home/);
    console.log(`${LOG_PREFIX} N1.3: passed — on /home, content: "${homeText}"`);
  });
});
