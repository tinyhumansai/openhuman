// [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough.
// This entire test file covered welcome-lock UI features (filteredThreads during
// lockdown, resolveThreadDisplayTitle "Onboarding" override, sidebar clamping,
// delete button hidden for welcome thread, etc.) that are no longer present
// in the codebase after the welcome-agent → Joyride migration.
//
// All tests are disabled to avoid failures on removed behavior. The file is
// preserved so the removal is visible in git history and reviewers can verify
// what was intentionally dropped.
import { describe, it } from 'vitest';

// [#1123] Placeholder to satisfy Vitest's "no test suite found" requirement.
// All real tests from this file were removed with the welcome-lock feature (#1123).
describe('[#1123] Conversations welcome-lock tests — removed', () => {
  it.skip('all welcome-lock tests disabled — feature removed in #1123', () => {
    // see above
  });
});
