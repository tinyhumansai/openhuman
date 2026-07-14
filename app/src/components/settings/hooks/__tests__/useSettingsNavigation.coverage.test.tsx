/**
 * Coverage-focused tests for useSettingsNavigation.
 *
 * The two-pane settings restructure retired breadcrumb navigation (the
 * `breadcrumbs` field is now always empty — the sidebar replaced the trail) and
 * collapsed the old section-hub pages (`ai`, `agents-settings`, `features`,
 * `notifications-hub`, `crypto`) into leaf panels reachable from the sidebar.
 * These tests now cover:
 *  - Exact-match route resolution (no substring collisions).
 *  - Leaf routes resolving to their own registry id.
 *  - Retired hub slugs and unknown slugs resolving to 'home'.
 *  - Breadcrumbs always being empty.
 */
import { screen } from '@testing-library/react';
import { describe, expect, test } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import { useSettingsNavigation } from '../useSettingsNavigation';

/** Renders breadcrumb labels and the currentRoute for assertion. */
const NavigationProbe = () => {
  const { breadcrumbs, currentRoute } = useSettingsNavigation();
  return (
    <div>
      <div data-testid="breadcrumbs">{breadcrumbs.map(b => b.label).join(' > ')}</div>
      <div data-testid="current-route">{currentRoute}</div>
    </div>
  );
};

const expectRoute = (path: string, route: string) => {
  renderWithProviders(<NavigationProbe />, { initialEntries: [path] });
  expect(screen.getByTestId('current-route')).toHaveTextContent(route);
  // Breadcrumbs are retired across the board — always empty.
  expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('');
};

// ---------------------------------------------------------------------------
// home root
// ---------------------------------------------------------------------------

describe('home route', () => {
  test('/settings resolves to home with empty breadcrumbs', () => {
    expectRoute('/settings', 'home');
  });
});

// ---------------------------------------------------------------------------
// Leaf routes resolve to their own registry id (representative per section)
// ---------------------------------------------------------------------------

describe('account section leaves', () => {
  test('privacy resolves to privacy', () => expectRoute('/settings/privacy', 'privacy'));
  test('security resolves to security', () => expectRoute('/settings/security', 'security'));
  test('team resolves to team', () => expectRoute('/settings/team', 'team'));
});

describe('ai section leaves', () => {
  test('llm resolves to llm', () => expectRoute('/settings/llm', 'llm'));
  test('voice resolves to voice', () => expectRoute('/settings/voice', 'voice'));
});

describe('agents section leaves', () => {
  test('agent-access resolves to agent-access', () =>
    expectRoute('/settings/agent-access', 'agent-access'));
});

describe('features section leaves', () => {
  test('tools resolves to tools', () => expectRoute('/settings/tools', 'tools'));
  test('companion resolves to companion', () => expectRoute('/settings/companion', 'companion'));
});

describe('integrations (retired)', () => {
  // The Integrations settings section was removed — the slug redirects to the
  // Connections page, so it no longer resolves to a registry entry.
  test('integrations resolves to home', () => expectRoute('/settings/integrations', 'home'));
});

describe('notifications', () => {
  test('notifications resolves to notifications', () =>
    expectRoute('/settings/notifications', 'notifications'));
});

describe('crypto section leaves', () => {
  test('recovery-phrase resolves to recovery-phrase', () =>
    expectRoute('/settings/recovery-phrase', 'recovery-phrase'));
  test('wallet-balances resolves to wallet-balances', () =>
    expectRoute('/settings/wallet-balances', 'wallet-balances'));
});

describe('developer section leaves', () => {
  test('cron-jobs resolves to cron-jobs', () => expectRoute('/settings/cron-jobs', 'cron-jobs'));
  // 'intelligence' was retired with the Brain Knowledge & Memory cleanup — the
  // old settings slug now falls through to home (redirect handled at the
  // route-elements layer, which bounces it to /brain).
  test('intelligence (retired) resolves to home', () =>
    expectRoute('/settings/intelligence', 'home'));
  test('developer-options resolves to developer-options', () =>
    expectRoute('/settings/developer-options', 'developer-options'));
});

// ---------------------------------------------------------------------------
// Retired hub slugs and unknown slugs resolve to home
// ---------------------------------------------------------------------------

describe('retired hub slugs resolve to home', () => {
  test('ai (retired hub) resolves to home', () => expectRoute('/settings/ai', 'home'));
  test('agents-settings (retired hub) resolves to home', () =>
    expectRoute('/settings/agents-settings', 'home'));
  test('features (retired hub) resolves to home', () => expectRoute('/settings/features', 'home'));
  test('notifications-hub (retired hub) resolves to home', () =>
    expectRoute('/settings/notifications-hub', 'home'));
  test('crypto (retired hub) resolves to home', () => expectRoute('/settings/crypto', 'home'));
});

describe('unknown / removed routes', () => {
  test('"messaging" route (removed) resolves to home', () =>
    expectRoute('/settings/messaging', 'home'));
  test('completely unknown slug resolves to home', () =>
    expectRoute('/settings/not-a-real-route', 'home'));
});

// ---------------------------------------------------------------------------
// Exact-match: no substring collision between "voice" and "voice-debug"
// ---------------------------------------------------------------------------

describe('no substring collision', () => {
  test('/settings/voice resolves to voice', () => {
    // Exact first-segment extraction: "voice" resolves to the voice leaf and
    // is not confused with any longer developer route.
    expectRoute('/settings/voice', 'voice');
  });

  test('/settings/voice-debug (retired) resolves to home', () => {
    // The voice-debug developer page was removed; its slug no longer resolves
    // to a registry entry (the route now redirects out).
    expectRoute('/settings/voice-debug', 'home');
  });
});
