import { waitForApp } from '../helpers/app-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash, waitForHomePage } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

describe('Coding-agent session memory', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp('e2e-coding-session-memory');
    // resetApp owns the complete auth and onboarding flow. Triggering another
    // deep link here races its asynchronous post-login redirect with the
    // Sources navigation below; wait for resetApp's settled Chat surface.
    await waitForHomePage();
    await navigateViaHash('/brain?tab=sources');
  });

  after(async () => {
    await stopMockServer();
  });

  it('surfaces Codex and Claude Code as private local memory sources', async () => {
    const card = await $('[data-testid="coding-sessions-card"]');
    await card.waitForDisplayed({ timeout: 20_000 });
    expect(await card.getText()).toContain('Coding-agent sessions');
    await expect($('[data-testid="coding-session-source-claude_code"]')).toBeDisplayed();
    await expect($('[data-testid="coding-session-source-codex"]')).toBeDisplayed();
    await expect($('[data-testid="coding-sessions-ingest"]')).toBeDisplayed();
  });
});
