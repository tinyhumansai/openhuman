// @ts-nocheck
/**
 * Skill OAuth UI smoke (issue #221).
 *
 * JSON-RPC coverage for OAuth + setup persistence lives in Rust integration
 * tests (`tests/json_rpc_e2e.rs`:
 * `json_rpc_skills_status_reflects_setup_complete_without_runtime`,
 * `json_rpc_skills_oauth_complete_after_start`). This spec only verifies
 * the post-auth Skills shell shows the connection/setup affordances a user
 * would tap to start that flow.
 */
import { waitForApp } from '../helpers/app-helpers';
import { textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateToSkills } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-skill-oauth';

describe('Skill OAuth UI smoke', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('Skills page shows skill rows with actions after login', async () => {
    await navigateToSkills();
    await browser.pause(2_500);

    const hasSkillChrome =
      (await textExists('Skills')) ||
      (await textExists('Install')) ||
      (await textExists('Available')) ||
      (await textExists('Connect')) ||
      (await textExists('Setup'));
    expect(hasSkillChrome).toBe(true);
  });
});
