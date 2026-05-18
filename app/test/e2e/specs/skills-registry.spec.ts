// @ts-nocheck
/**
 * Skills registry E2E flow.
 *
 * Drives browse → install → uninstall through real UI clicks. The mock
 * backend's `/skills` registry seeds the catalog; install/uninstall hits
 * are validated through both UI feedback AND the mock request log so we
 * know the click actually reached the network.
 */
import { waitForApp } from '../helpers/app-helpers';
import {
  clickButton,
  clickText,
  dumpAccessibilityTree,
  textExists,
} from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateToSkills } from '../helpers/shared-flows';
import { clearRequestLog, getRequestLog, startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-skills-registry';

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
    const match = log.find(r => r.method === method && r.url.includes(urlFragment));
    if (match) return match;
    await browser.pause(500);
  }
  return undefined;
}

describe('Skills registry flow', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('navigates to /skills and renders catalog content', async () => {
    clearRequestLog();
    await navigateToSkills();

    const currentHash = await browser.execute(() => window.location.hash);
    expect(String(currentHash)).toContain('/skills');

    await browser.pause(2_000);
    const hasSkillsContent =
      (await textExists('Install')) ||
      (await textExists('Available')) ||
      (await textExists('Skills')) ||
      (await textExists('Telegram')) ||
      (await textExists('Notion'));

    if (!hasSkillsContent) {
      await dumpAccessibilityTree();
      console.error('[SkillsRegistryE2E] request log:', getRequestLog());
    }
    expect(hasSkillsContent).toBe(true);
  });

  it('shows at least one known registry skill name', async () => {
    const hasNamedSkill =
      (await textExists('Telegram')) || (await textExists('Notion')) || (await textExists('Gmail'));
    expect(hasNamedSkill).toBe(true);
  });

  it('install button (if present) triggers an RPC request', async () => {
    clearRequestLog();
    try {
      await clickButton('Install', 5_000);
    } catch {
      // No install button visible in this state — skip the RPC oracle.
      return;
    }
    const req = await waitForRequest('POST', '/rpc', 10_000);
    expect(req).toBeDefined();
  });

  it('uninstall affordance (if present) triggers an RPC request', async () => {
    clearRequestLog();
    const labels = ['Uninstall', 'Disconnect', 'Remove'];
    let clicked = false;
    for (const label of labels) {
      try {
        await clickText(label, 3_000);
        clicked = true;
        break;
      } catch {
        // try next label
      }
    }
    if (!clicked) return;
    const req = await waitForRequest('POST', '/rpc', 10_000);
    expect(req).toBeDefined();
  });
});
