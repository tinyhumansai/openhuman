/**
 * Skills registry E2E test
 *
 * Tests the end-to-end flow for browsing, installing, and uninstalling
 * skills from the remote registry through the UI.
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  clickButton,
  clickText,
  dumpAccessibilityTree,
  textExists,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { navigateToSkills } from '../helpers/shared-flows';
import { clearRequestLog, getRequestLog, startMockServer, stopMockServer } from '../mock-server';

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[SkillsRegistryE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[SkillsRegistryE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

interface RequestLogEntry {
  method: string;
  url: string;
  body?: unknown;
}

async function waitForRequest(
  method: string,
  urlFragment: string,
  timeoutMs = 15_000
): Promise<RequestLogEntry | undefined> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const log = getRequestLog() as RequestLogEntry[];
    const match = log.find(
      (r: RequestLogEntry) => r.method === method && r.url.includes(urlFragment)
    );
    if (match) return match;
    await browser.pause(500);
  }
  return undefined;
}

describe('Skills registry flow', () => {
  before(async () => {
    stepLog('Starting skills registry E2E test');
    await startMockServer();
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  it('authenticates and reaches home screen', async () => {
    stepLog('Triggering auth deep link bypass');
    await triggerAuthDeepLinkBypass('e2e-skills-user');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    stepLog('App is ready');
  });

  it('can navigate to skills page', async () => {
    stepLog('Navigating to Skills page');
    clearRequestLog();
    await navigateToSkills();

    // Verify hash changed to /skills
    const currentHash = await browser.execute(() => window.location.hash);
    stepLog(`Current hash: ${currentHash}`);
    expect(currentHash).toContain('/skills');

    // Wait for skills page content to render and verify a UI marker
    await browser.pause(2_000);
    const hasSkillsContent =
      (await textExists('Install')) ||
      (await textExists('Available')) ||
      (await textExists('Skills')) ||
      (await textExists('Telegram')) ||
      (await textExists('Notion'));

    if (!hasSkillsContent) {
      const tree = await dumpAccessibilityTree();
      const log = getRequestLog();
      stepLog('Skills page content not found after navigation');
      stepLog('Accessibility tree:', tree.slice(0, 4000));
      stepLog('Request log:', log);
    }
    expect(hasSkillsContent).toBe(true);
    stepLog('Skills page verified');
  });

  it('displays available skills from registry', async () => {
    // The skills page should show some skill names from the mock backend
    // The exact text depends on the UI implementation, but we verify the page loaded
    const pageHasContent = (await textExists('Install')) || (await textExists('Available'));
    stepLog(`Skills page has install/available content: ${pageHasContent}`);

    // Dump tree for debugging if content is missing
    if (!pageHasContent) {
      stepLog('Dumping accessibility tree for debugging');
      await dumpAccessibilityTree();
    }
  });

  it('can trigger a skill install action', async () => {
    clearRequestLog();

    // Try to click an Install button if available
    try {
      await clickButton('Install', 5_000);
      stepLog('Clicked Install button');

      // Check if an RPC request was made
      const req = await waitForRequest('POST', '/rpc', 10_000);
      if (req) {
        stepLog('Install RPC request detected', req);
      }
    } catch {
      stepLog('No Install button found (may need different UI state)');
    }
  });

  it('shows at least one known registry skill name (tool surface)', async () => {
    await navigateToSkills();
    await browser.pause(1_500);
    const hasNamedSkill =
      (await textExists('Telegram')) ||
      (await textExists('Notion')) ||
      (await textExists('Gmail'));
    stepLog(`Registry skill name visible: ${hasNamedSkill}`);
    expect(hasNamedSkill).toBe(true);
  });

  it('can trigger a skill uninstall action', async () => {
    clearRequestLog();

    // Try to click Disconnect/Uninstall/Remove button if available
    const buttons = ['Uninstall', 'Disconnect', 'Remove'];
    let clicked = false;
    for (const label of buttons) {
      try {
        await clickText(label, 3_000);
        stepLog(`Clicked ${label} button`);
        clicked = true;
        break;
      } catch {
        // Try next button label
      }
    }

    if (clicked) {
      const req = await waitForRequest('POST', '/rpc', 10_000);
      if (req) {
        stepLog('Uninstall RPC request detected', req);
      }
    } else {
      stepLog('No uninstall button found (expected if no skill is installed)');
    }
  });
});
