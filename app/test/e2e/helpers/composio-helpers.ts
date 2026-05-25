/**
 * Shared helpers for Composio connector E2E specs.
 *
 * All helpers are platform-agnostic (tauri-driver + Appium Mac2) and
 * follow the same patterns established in composio-triggers-flow.spec.ts
 * and the existing shared-flows / element-helpers modules.
 */
import { setMockBehavior } from '../mock-server';
import { textExists, waitForText } from './element-helpers';
import { navigateToHome, navigateToSkills, waitForHomePage } from './shared-flows';

const LOG = '[ComposioHelpers]';

// ---------------------------------------------------------------------------
// Seed helpers — set mock behavior knobs before navigation
// ---------------------------------------------------------------------------

/**
 * Seed a single Composio connection into the mock backend.
 *
 * Sets the `composioConnections` behavior knob with a single entry for the
 * given toolkit.  Subsequent calls overwrite any previous seed — isolate
 * specs by calling this in `beforeEach` or at the start of each test.
 */
export function seedComposioConnection(
  toolkit: string,
  status: 'ACTIVE' | 'FAILED' | 'EXPIRED' | 'CONNECTING',
  connectionId: string = 'c-e2e'
): void {
  setMockBehavior('composioConnections', JSON.stringify([{ id: connectionId, toolkit, status }]));
}

/**
 * Seed the list of available Composio toolkits shown on the Skills page.
 *
 * Sets the `composioToolkits` behavior knob to the given slugs array.
 */
export function seedComposioToolkits(slugs: string[]): void {
  setMockBehavior('composioToolkits', JSON.stringify(slugs));
}

// ---------------------------------------------------------------------------
// Navigation + UI assertion helpers
// ---------------------------------------------------------------------------

/**
 * Navigate to /skills and wait until the connector card with the given
 * display name is visible.
 *
 * Throws (via waitForText) if the card is not visible within the timeout.
 */
export async function assertConnectorCardVisible(name: string, timeout = 15_000): Promise<void> {
  await navigateToSkills();
  await waitForText(name, timeout);
  console.log(`${LOG} connector card visible: "${name}"`);
}

/**
 * Click a connector card by display name, then wait for the modal header
 * to appear.  The modal header text is either "Connect <name>", "Manage
 * <name>", or "Reconnect <name>" depending on connection state.
 *
 * Returns the modal header text that was found, or null when none of the
 * candidates appeared within the timeout (so callers that can tolerate a
 * missing modal don't have to wrap in try/catch).
 */
export async function openConnectorModal(name: string, timeout = 15_000): Promise<string | null> {
  console.log(`${LOG} opening connector modal for "${name}"`);
  const candidates = [
    `Connect ${name}`,
    `Manage ${name}`,
    `Reconnect ${name}`,
    `${name} authorization expired`,
    `${name} is connected`,
    'Disconnect',
  ];

  const ensureModalOpen = async (): Promise<boolean> =>
    browser.execute((connectorName: string) => {
      const dialog = document.querySelector('[role="dialog"]');
      if (dialog) return true;
      const exactButton = Array.from(document.querySelectorAll('button')).find(btn => {
        const label = btn.getAttribute('aria-label') ?? '';
        const title = btn.getAttribute('title') ?? '';
        const text = btn.textContent ?? '';
        return (
          label.includes(connectorName) ||
          title.includes(connectorName) ||
          text.includes(connectorName)
        );
      }) as HTMLButtonElement | undefined;
      if (!exactButton) return false;
      exactButton.click();
      return false;
    }, name);

  // Click once up front. If the modal appears, stop trying to re-click the
  // underlying card; the backdrop will intercept any later coordinate clicks.
  await waitForText(name, timeout);
  await ensureModalOpen();

  const deadline = Date.now() + timeout;
  let lastReopenAt = 0;
  while (Date.now() < deadline) {
    for (const candidate of candidates) {
      if (await textExists(candidate)) {
        console.log(`${LOG} modal opened: "${candidate}"`);
        return candidate;
      }
    }
    const modalVisible = await browser
      .execute(() => Boolean(document.querySelector('[role="dialog"]')))
      .catch(() => false);
    if (!modalVisible && Date.now() - lastReopenAt > 1_000) {
      await ensureModalOpen();
      lastReopenAt = Date.now();
    }
    // @ts-expect-error -- browser global is injected by WDIO at runtime, not typed in this env
    await browser.pause(250);
  }

  console.log(`${LOG} modal for "${name}" did not open within timeout`);
  return null;
}

/**
 * Assert the modal is in a given phase by checking UI markers.
 *
 * Phase markers:
 *   idle       — Connect button present (no active connection)
 *   connected  — "is connected" or Disconnect button visible
 *   expired    — "authorization expired" text visible
 *   error      — error UI present (coral-coloured error block)
 */
export async function assertModalPhase(
  phase: 'idle' | 'connected' | 'expired' | 'error',
  name: string,
  timeout = 10_000
): Promise<void> {
  const deadline = Date.now() + timeout;

  const phaseMarkers: Record<string, string[]> = {
    idle: [`Connect ${name}`, 'Connect'],
    connected: ['Disconnect', 'is connected'],
    expired: ['authorization expired', 'Reconnect'],
    error: ['Something went wrong', 'Authorization failed', 'dismissAll'],
  };

  const markers = phaseMarkers[phase] ?? [];
  while (Date.now() < deadline) {
    for (const marker of markers) {
      if (await textExists(marker)) {
        console.log(`${LOG} modal phase "${phase}" confirmed via marker: "${marker}"`);
        return;
      }
    }
    // @ts-expect-error -- browser global is injected by WDIO at runtime, not typed in this env
    await browser.pause(400);
  }

  throw new Error(
    `assertModalPhase: phase "${phase}" for "${name}" not confirmed within ${timeout}ms — no marker found in [${markers.join(', ')}]`
  );
}

/**
 * Assert that the user session is still alive (not logged out) by navigating
 * to /home and waiting for home page content.
 *
 * This is the key guard for the "401 on composio routes must NOT log user
 * out" class of regressions (#2285, #2286).
 */
export async function assertSessionNotNuked(timeout = 20_000): Promise<void> {
  console.log(`${LOG} asserting session is intact — navigating to /home`);
  await navigateToHome();
  const marker = await waitForHomePage(timeout);
  if (!marker) {
    throw new Error(`assertSessionNotNuked: Home page not reached — user may have been logged out`);
  }
  console.log(`${LOG} session intact, home page marker: "${marker}"`);
}

/**
 * Inject a mock HTTP fault on all Composio routes by setting the
 * composioExecuteFails / composioDeleteFails / composioSyncFails behavior
 * knobs to trigger the given status code.
 *
 * Supported status codes: 400, 500.
 * The mock route handlers interpret knob value '400' → HTTP 400 and '500' → HTTP 500.
 */
export function injectComposioFault(statusCode: 400 | 500): void {
  const value = String(statusCode);
  setMockBehavior('composioExecuteFails', value);
  setMockBehavior('composioDeleteFails', value);
  setMockBehavior('composioSyncFails', value);
  console.log(`${LOG} injected composio fault: status=${statusCode}`);
}
