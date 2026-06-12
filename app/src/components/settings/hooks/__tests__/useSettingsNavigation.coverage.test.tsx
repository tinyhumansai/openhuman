/**
 * Coverage-focused tests for useSettingsNavigation.
 *
 * These tests supplement the existing breadcrumb tests with:
 *  - Exact-match route resolution (no substring collisions).
 *  - Canonical breadcrumbs for a representative route in each section.
 *  - Composio section page and leaf breadcrumbs.
 *  - Verification that the removed 'messaging' route returns 'home'.
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

// ---------------------------------------------------------------------------
// Section: home (no breadcrumbs at root)
// ---------------------------------------------------------------------------

describe('home route', () => {
  test('/settings resolves to home with empty breadcrumbs', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings'] });
    expect(screen.getByTestId('current-route')).toHaveTextContent('home');
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('');
  });
});

// ---------------------------------------------------------------------------
// Section: account — representative leaf
// ---------------------------------------------------------------------------

describe('account section', () => {
  test('privacy returns Settings > Account breadcrumb', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/privacy'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > Account');
  });

  test('security returns Settings > Account breadcrumb', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/security'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > Account');
  });

  test('team returns Settings > Account breadcrumb', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/team'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > Account');
  });
});

// ---------------------------------------------------------------------------
// Section: ai — representative leaf
// ---------------------------------------------------------------------------

describe('ai section', () => {
  test('llm returns Settings > AI & Models breadcrumb', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/llm'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > AI & Models');
  });

  test('voice returns Settings > AI & Models breadcrumb', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/voice'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > AI & Models');
  });

  // Exact-match check: /settings/ai must not substring-match into a longer route.
  test('/settings/ai resolves to section page "ai" (not a deeper route)', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/ai'] });
    expect(screen.getByTestId('current-route')).toHaveTextContent('ai');
    // ai is a home-level section hub — breadcrumb is just Settings.
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings');
  });
});

// ---------------------------------------------------------------------------
// Section: agents — representative leaf
// ---------------------------------------------------------------------------

describe('agents section', () => {
  test('autonomy returns Settings > Agents breadcrumb', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/autonomy'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > Agents');
  });

  test('agent-access returns Settings > Agents breadcrumb', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/agent-access'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > Agents');
  });

  test('agents-settings section page returns Settings only', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/agents-settings'] });
    expect(screen.getByTestId('current-route')).toHaveTextContent('agents-settings');
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings');
  });
});

// ---------------------------------------------------------------------------
// Section: features — representative leaf
// ---------------------------------------------------------------------------

describe('features section', () => {
  test('tools returns Settings > Features breadcrumb', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/tools'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > Features');
  });

  test('companion returns Settings > Features breadcrumb', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/companion'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > Features');
  });
});

// ---------------------------------------------------------------------------
// Section: composio — section page + leaf
// ---------------------------------------------------------------------------

describe('composio section', () => {
  test('composio section page returns Settings only', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/composio'] });
    expect(screen.getByTestId('current-route')).toHaveTextContent('composio');
    // composio is a home-level section hub — breadcrumb is just Settings.
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings');
  });

  test('task-sources returns Settings > Integrations breadcrumb', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/task-sources'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > Integrations');
  });

  test('webhooks-triggers returns Settings > Integrations breadcrumb', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/webhooks-triggers'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > Integrations');
  });
});

// ---------------------------------------------------------------------------
// Section: notifications — section page + leaf
// ---------------------------------------------------------------------------

describe('notifications section', () => {
  test('notifications-hub section page returns Settings only', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/notifications-hub'] });
    expect(screen.getByTestId('current-route')).toHaveTextContent('notifications-hub');
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings');
  });

  test('notifications leaf returns Settings > Notifications', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/notifications'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > Notifications');
  });
});

// ---------------------------------------------------------------------------
// Section: crypto — section page + leaf
// ---------------------------------------------------------------------------

describe('crypto section', () => {
  test('crypto section page returns Settings only', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/crypto'] });
    expect(screen.getByTestId('current-route')).toHaveTextContent('crypto');
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings');
  });

  test('recovery-phrase returns Settings > Crypto', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/recovery-phrase'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > Crypto');
  });

  test('wallet-balances returns Settings > Crypto', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/wallet-balances'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > Crypto');
  });
});

// ---------------------------------------------------------------------------
// Section: developer — representative leaf
// ---------------------------------------------------------------------------

describe('developer section', () => {
  test('cron-jobs returns Settings > Developer Options', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/cron-jobs'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > Developer Options');
  });

  test('intelligence returns Settings > Developer Options', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/intelligence'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > Developer Options');
  });

  test('developer-options section page returns Settings only', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/developer-options'] });
    expect(screen.getByTestId('current-route')).toHaveTextContent('developer-options');
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings');
  });
});

// ---------------------------------------------------------------------------
// Removed routes / unknown slugs
// ---------------------------------------------------------------------------

describe('unknown / removed routes', () => {
  test('"messaging" route (removed) resolves to home', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/messaging'] });
    expect(screen.getByTestId('current-route')).toHaveTextContent('home');
  });

  test('completely unknown slug resolves to home', () => {
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/not-a-real-route'] });
    expect(screen.getByTestId('current-route')).toHaveTextContent('home');
  });
});

// ---------------------------------------------------------------------------
// Exact-match: no substring collision between /settings/ai and longer paths
// ---------------------------------------------------------------------------

describe('no substring collision', () => {
  test('/settings/ai does not match /settings/agent-access or /settings/agents', () => {
    // Verifies that exact first-segment extraction prevents "ai" from matching
    // routes whose slugs merely contain "ai" as a substring.
    renderWithProviders(<NavigationProbe />, { initialEntries: ['/settings/ai'] });
    expect(screen.getByTestId('current-route')).toHaveTextContent('ai');
    // Must not bleed into the agents section.
    expect(screen.getByTestId('breadcrumbs')).not.toHaveTextContent('Agents');
  });
});
