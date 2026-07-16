import { waitForApp } from '../helpers/app-helpers';
import { navigateViaHash } from '../helpers/shared-flows';

describe('Coding-agent session memory', () => {
  before(async () => {
    await waitForApp();
    await navigateViaHash('/brain?tab=sources');
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
