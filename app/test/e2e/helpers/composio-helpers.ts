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
  // `INITIATED` / `PENDING` are the statuses Composio reports while an OAuth
  // handoff is still open — what `deriveComposioState` maps to `pending` and
  // the modal renders as the waiting phase.
  status: 'ACTIVE' | 'FAILED' | 'EXPIRED' | 'CONNECTING' | 'INITIATED' | 'PENDING',
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
export async function openConnectorModal(
  name: string,
  timeout = 15_000,
  /** Optional tile-level status text to wait for before clicking (e.g. 'Auth expired').
   * Ensures connection data has loaded so the modal opens in the correct phase. */
  waitForTileStatus?: string
): Promise<string | null> {
  console.log(`${LOG} opening connector modal for "${name}"`);
  // Click the connector's OWN control, not merely a button that mentions it.
  //
  // This used to take the first button whose aria-label/title/text `includes`
  // the connector name, under a variable called `exactButton` — the name
  // described an intent the predicate did not implement. A bare substring match
  // collides with any unrelated chrome carrying the same word, and `.find()`
  // returns the first such button in DOM order, which is whichever the shell
  // renders earliest.
  //
  // That is not hypothetical and it is why this helper looked "flaky" for
  // exactly one connector. `SidebarHeader.tsx:95-103` renders a community-invite
  // button with `aria-label="Join our Discord"` on every page, ahead of the page
  // content. For `name = 'Discord'`, `'Join our Discord'.includes('Discord')` is
  // true, so the helper clicked the invite — which calls `openUrl` and hands off
  // to the system browser — instead of the Discord connector tile. The modal
  // never opened, every retry re-clicked the same wrong button, and the failure
  // looked like slow modal discovery. `Jira`, `GitHub` and `Gmail` have no
  // equivalent twin in the shell, which is why three sibling specs calling this
  // helper with the identical signature never saw it.
  //
  // Match the tile's own action labels exactly (trimmed, case-insensitively),
  // falling back to a substring match only when none is present — preserving the
  // old reach for tiles whose label this list does not yet name, without letting
  // unrelated chrome win by being earlier in the document.
  //
  // Deliberately NOT reusing `candidates` for this: that list is the set of
  // strings that indicate the modal is ALREADY open, and includes in-modal text
  // like `Disconnect`. Those are the right things to detect and the wrong things
  // to click.
  const ensureModalOpen = async (): Promise<boolean> =>
    browser.execute(
      (connectorName: string, accepted: string[]) => {
        const dialog = document.querySelector('[role="dialog"]');
        if (dialog) return true;
        const buttons = Array.from(document.querySelectorAll('button'));
        const labelsOf = (btn: Element): string[] =>
          [
            btn.getAttribute('aria-label') ?? '',
            btn.getAttribute('title') ?? '',
            btn.textContent ?? '',
          ].map(value => value.trim().toLowerCase());
        const wanted = accepted.map(value => value.trim().toLowerCase());

        const exact = buttons.find(btn => labelsOf(btn).some(value => wanted.includes(value)));
        const target = (exact ??
          buttons.find(btn =>
            labelsOf(btn).some(value => value.includes(connectorName.toLowerCase()))
          )) as HTMLButtonElement | undefined;
        if (!target) return false;
        target.click();
        return false;
      },
      name,
      [`Connect ${name}`, `Manage ${name}`, `Reconnect ${name}`]
    );

  const readModalHeading = async (): Promise<string | null> =>
    browser.execute((connectorName: string) => {
      const dialog = document.querySelector<HTMLElement>('[role="dialog"]');
      if (!dialog || dialog.getClientRects().length === 0) return null;
      const heading = dialog.querySelector<HTMLElement>(
        '[role="heading"], h1, h2, h3, [id$="-title"]'
      );
      const title = heading?.textContent?.trim();
      return title && title.toLowerCase().includes(connectorName.toLowerCase()) ? title : null;
    }, name);

  // Click once up front. If the modal appears, stop trying to re-click the
  // underlying card; the backdrop will intercept any later coordinate clicks.
  await waitForText(name, timeout);
  // If a tile status is expected (e.g. 'Auth expired'), wait for it before
  // clicking so the modal opens with connection data already loaded.
  if (waitForTileStatus) {
    await browser.waitUntil(
      async () =>
        browser.execute(
          (connectorName: string, expectedStatus: string) => {
            const tiles = Array.from(
              document.querySelectorAll<HTMLButtonElement>(
                'button[data-testid^="skill-install-composio-"]'
              )
            );
            const tile = tiles.find(button => {
              const label = button.getAttribute('aria-label') ?? '';
              const visibleName = button.querySelector('span')?.textContent?.trim() ?? '';
              return (
                label.toLowerCase().startsWith(`${connectorName.toLowerCase()},`) ||
                visibleName.toLowerCase() === connectorName.toLowerCase()
              );
            });
            if (!tile) return false;
            const state = `${tile.getAttribute('aria-label') ?? ''} ${tile.textContent ?? ''}`;
            return state.toLowerCase().includes(expectedStatus.toLowerCase());
          },
          name,
          waitForTileStatus
        ),
      {
        timeout,
        interval: 300,
        timeoutMsg: `Connector tile "${name}" did not show the requested status "${waitForTileStatus}"`,
      }
    );
  }
  await ensureModalOpen();

  const deadline = Date.now() + timeout;
  let lastReopenAt = 0;
  while (Date.now() < deadline) {
    const heading = await readModalHeading();
    if (heading) {
      console.log(`${LOG} modal opened: "${heading}"`);
      return heading;
    }
    const modalVisible = await browser
      .execute(() => Boolean(document.querySelector('[role="dialog"]')))
      .catch(() => false);
    if (!modalVisible && Date.now() - lastReopenAt > 1_000) {
      await ensureModalOpen();
      lastReopenAt = Date.now();
    }
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
 *   waiting    — OAuth handoff open: waiting copy / Cancel connection
 *   connected  — "is connected" or Disconnect button visible
 *   expired    — "authorization expired" text visible
 *   error      — error UI present (coral-coloured error block)
 */
export async function assertModalPhase(
  phase: 'idle' | 'connected' | 'expired' | 'error' | 'waiting',
  name: string,
  timeout = 10_000
): Promise<void> {
  const deadline = Date.now() + timeout;

  const phaseMarkers: Record<string, string[]> = {
    idle: [`Connect ${name}`, 'Connect'],
    connected: ['Disconnect', 'is connected'],
    expired: ['authorization expired', 'Reconnect to re-enable', 'Reconnect'],
    error: ['Something went wrong', 'Authorization failed', 'dismissAll'],
    waiting: ['Waiting for', 'Cancel connection', 'Reopen browser'],
  };

  const markers = phaseMarkers[phase] ?? [];
  while (Date.now() < deadline) {
    for (const marker of markers) {
      if (await textExists(marker)) {
        console.log(`${LOG} modal phase "${phase}" confirmed via marker: "${marker}"`);
        return;
      }
    }
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
 * composioExecuteFails / composioDeleteFails behavior knobs to trigger the given status code.
 *
 * Supported status codes: 400, 500.
 * The mock route handlers interpret knob value '400' → HTTP 400 and '500' → HTTP 500.
 */
export function injectComposioFault(statusCode: 400 | 500): void {
  const value = String(statusCode);
  setMockBehavior('composioExecuteFails', value);
  setMockBehavior('composioDeleteFails', value);
  console.log(`${LOG} injected composio fault: status=${statusCode}`);
}
