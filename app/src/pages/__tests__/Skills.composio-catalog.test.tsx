import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import '../../test/mockDefaultSkillStatusHooks';
import { renderWithProviders } from '../../test/test-utils';
import Skills from '../Skills';

let composioRefresh = vi.fn();
let composioError: string | null = null;
let composioLoading = false;
let composioToolkits: string[] = [];
let composioCatalogByToolkit = new Map();
let composioConnectionByToolkit = new Map();
let composioConnectionsByToolkitOverride: Map<string, unknown[]> | null = null;
let sessionToken = 'jwt-abc';
let composioModeStatus = { result: { mode: 'backend', api_key_set: true }, logs: [] };
// CodeRabbit on #2361: failure-path coverage for the agent-ready
// RPC requires overriding the hook's state per test. Default state
// keeps Preview badges off (loading=true) so legacy assertions on
// this file don't drift.
let agentReadyState: { agentReady: Set<string>; loading: boolean; error: string | null } = {
  agentReady: new Set<string>(),
  loading: true,
  error: null,
};

vi.mock('../../hooks/useChannelDefinitions', () => ({
  useChannelDefinitions: () => ({ definitions: [], loading: false, error: null }),
}));

vi.mock('../../lib/skills/skillsApi', () => ({
  installSkill: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../lib/skills/hooks', () => ({
  useAvailableSkills: () => ({ skills: [], loading: false, refresh: vi.fn() }),
}));

vi.mock('../../lib/composio/hooks', () => ({
  useComposioIntegrations: () => ({
    toolkits: composioToolkits,
    catalogByToolkit: composioCatalogByToolkit,
    connectionByToolkit: composioConnectionByToolkit,
    connectionsByToolkit:
      composioConnectionsByToolkitOverride ??
      new Map(Array.from(composioConnectionByToolkit.entries()).map(([k, v]) => [k, [v]])),
    refresh: composioRefresh,
    loading: composioLoading,
    error: composioError,
  }),
  // Issue #2283 / CodeRabbit on #2361: Skills.tsx consumes
  // useAgentReadyComposioToolkits. We route through a module-level
  // `agentReadyState` so individual tests can override `loading` /
  // `error` to exercise the failure-fallback path.
  useAgentReadyComposioToolkits: () => agentReadyState,
}));

vi.mock('../../lib/coreState/store', async () => {
  const actual = await vi.importActual<typeof import('../../lib/coreState/store')>(
    '../../lib/coreState/store'
  );
  return { ...actual, getCoreStateSnapshot: () => ({ snapshot: { sessionToken } }) };
});

vi.mock('../../utils/tauriCommands', async () => {
  const actual = await vi.importActual<typeof import('../../utils/tauriCommands')>(
    '../../utils/tauriCommands'
  );
  return { ...actual, openhumanComposioGetMode: vi.fn(async () => composioModeStatus) };
});

describe('Skills page — Composio catalog fallback', () => {
  beforeEach(() => {
    composioRefresh = vi.fn();
    composioError = null;
    composioLoading = false;
    composioToolkits = [];
    composioCatalogByToolkit = new Map();
    composioConnectionByToolkit = new Map();
    composioConnectionsByToolkitOverride = null;
    sessionToken = 'jwt-abc';
    composioModeStatus = { result: { mode: 'backend', api_key_set: true }, logs: [] };
    agentReadyState = { agentReady: new Set<string>(), loading: true, error: null };
  });

  function openAppsTab() {
    fireEvent.click(screen.getByTestId('two-pane-nav-composio'));
  }

  it('shows known composio integrations in the integrations icon grid when the live toolkit list is empty', () => {
    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });
    openAppsTab();

    expect(screen.getByTestId('composio-integrations-card')).toBeInTheDocument();
    expect(screen.getByText('Discord')).toBeInTheDocument();
    expect(screen.getByText('Google Calendar')).toBeInTheDocument();
    expect(screen.getByText('Google Drive')).toBeInTheDocument();
    expect(screen.getByText('Gmail')).toBeInTheDocument();
    expect(screen.getByText('Google Sheets')).toBeInTheDocument();
    expect(screen.getByText('Facebook')).toBeInTheDocument();
    expect(screen.getByText('GitHub')).toBeInTheDocument();
    expect(screen.getByText('Instagram')).toBeInTheDocument();
    expect(screen.getByText('Linear')).toBeInTheDocument();
    expect(screen.getByText('Reddit')).toBeInTheDocument();
    expect(screen.getByText('Slack')).toBeInTheDocument();
    expect(screen.getByText('Supabase')).toBeInTheDocument();
    // Scope to the Integrations section so the assertion still catches a
    // missing Composio Zoom tile even though the Meeting bots card also
    // renders a "Zoom" entry on the same page.
    const integrationsSection = screen.getByTestId('composio-integrations-card');
    expect(within(integrationsSection as HTMLElement).getByText('Zoom')).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Other' })).not.toBeInTheDocument();
  });

  it('renders integrations from the live dynamic catalog when the backend provides one', () => {
    // When the backend ships the dynamic catalog, the grid is sourced from
    // it (names/categories from the entry), not the hardcoded fallback list.
    composioCatalogByToolkit = new Map([
      [
        'acme_crm',
        { slug: 'acme_crm', name: 'Acme CRM Dynamic', categories: ['crm'], enabled: true },
      ],
      [
        'gmail',
        { slug: 'gmail', name: 'Gmail Dynamic', categories: ['productivity'], enabled: true },
      ],
    ]);

    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });
    openAppsTab();

    const integrationsSection = screen.getByTestId('composio-integrations-card');
    // Names come from the dynamic catalog entry, overriding local metadata.
    expect(
      within(integrationsSection as HTMLElement).getByText('Acme CRM Dynamic')
    ).toBeInTheDocument();
    expect(
      within(integrationsSection as HTMLElement).getByText('Gmail Dynamic')
    ).toBeInTheDocument();
    // A hardcoded-only toolkit absent from the dynamic catalog must NOT
    // appear — proving the grid is driven by the backend, not KNOWN_*.
    expect(
      within(integrationsSection as HTMLElement).queryByText('Discord')
    ).not.toBeInTheDocument();
  });

  it('shows a loading skeleton (not the hardcoded list) while the catalog is still being fetched', () => {
    // #3933: during the in-flight fetch we must NOT flash the hardcoded
    // KNOWN_COMPOSIO_TOOLKITS list. Until the backend catalog lands the grid
    // renders a loading skeleton instead.
    composioLoading = true;
    composioCatalogByToolkit = new Map();
    composioToolkits = [];

    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });
    openAppsTab();

    const integrationsSection = screen.getByTestId('composio-integrations-card');
    // Loading skeleton is shown…
    expect(
      within(integrationsSection as HTMLElement).getByTestId('composio-integrations-loading')
    ).toBeInTheDocument();
    expect(
      within(integrationsSection as HTMLElement).queryAllByTestId('composio-skeleton-tile').length
    ).toBeGreaterThan(0);
    // …and none of the hardcoded fallback toolkits are rendered yet.
    expect(
      within(integrationsSection as HTMLElement).queryByText('Discord')
    ).not.toBeInTheDocument();
    expect(within(integrationsSection as HTMLElement).queryByText('Gmail')).not.toBeInTheDocument();
    expect(within(integrationsSection as HTMLElement).queryByText('Slack')).not.toBeInTheDocument();
  });

  it('falls back to the hardcoded list once the fetch finishes without a dynamic catalog', () => {
    // The hardcoded list is a *post-fetch* fallback: when loading completes and
    // the backend supplied no catalog (a failure or an older core), the grid
    // must still render so it is never empty.
    composioLoading = false;
    composioCatalogByToolkit = new Map();

    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });
    openAppsTab();

    const integrationsSection = screen.getByTestId('composio-integrations-card');
    expect(
      within(integrationsSection as HTMLElement).queryByTestId('composio-integrations-loading')
    ).not.toBeInTheDocument();
    expect(within(integrationsSection as HTMLElement).getByText('Discord')).toBeInTheDocument();
    expect(within(integrationsSection as HTMLElement).getByText('Gmail')).toBeInTheDocument();
  });

  it('shows the dynamic catalog (not the skeleton) even if a fetch is still in flight once entries exist', () => {
    // Defensive: if catalog entries are already present we render them rather
    // than the skeleton, even should `loading` still be true for a poll cycle.
    composioLoading = true;
    composioCatalogByToolkit = new Map([
      [
        'gmail',
        { slug: 'gmail', name: 'Gmail Dynamic', categories: ['productivity'], enabled: true },
      ],
    ]);

    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });
    openAppsTab();

    const integrationsSection = screen.getByTestId('composio-integrations-card');
    expect(
      within(integrationsSection as HTMLElement).queryByTestId('composio-integrations-loading')
    ).not.toBeInTheDocument();
    expect(
      within(integrationsSection as HTMLElement).getByText('Gmail Dynamic')
    ).toBeInTheDocument();
    // Hardcoded-only toolkit absent from the dynamic catalog stays hidden.
    expect(
      within(integrationsSection as HTMLElement).queryByText('Discord')
    ).not.toBeInTheDocument();
  });

  it('shows a stale/error state instead of disconnected toolkits when composio loading fails', () => {
    composioError = 'Backend unavailable';

    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });
    openAppsTab();

    expect(screen.getByText('Connections are showing stale status')).toBeInTheDocument();
    expect(screen.getByText('Backend unavailable')).toBeInTheDocument();

    const integrationsSection = screen.getByTestId('composio-integrations-card');
    const gmailTile = within(integrationsSection as HTMLElement).getByRole('button', {
      name: /Gmail.*Status unavailable/i,
    });
    expect(gmailTile).toBeInTheDocument();
    expect(within(gmailTile).getByText('Status unavailable')).toBeInTheDocument();

    fireEvent.click(screen.getAllByRole('button', { name: 'Try again' })[0]);
    expect(composioRefresh).toHaveBeenCalledTimes(1);
  });

  it('surfaces expired Composio auth as reconnectable from the Gmail tile', () => {
    composioToolkits = ['gmail'];
    composioConnectionByToolkit = new Map([
      ['gmail', { id: 'ca_expired', toolkit: 'gmail', status: 'EXPIRED' }],
    ]);

    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });
    openAppsTab();

    const integrationsSection = screen.getByTestId('composio-integrations-card');
    const gmailTile = within(integrationsSection as HTMLElement).getByRole('button', {
      name: /Gmail.*Auth expired.*Reconnect/i,
    });

    expect(within(gmailTile).getByText('Auth expired')).toBeInTheDocument();

    fireEvent.click(gmailTile);

    expect(screen.getByText(/Gmail authorization expired/i)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Reconnect Gmail/i })).toBeInTheDocument();
  });

  it('shows a multi-account count badge when a toolkit has more than one active connection', () => {
    composioToolkits = ['gmail'];
    composioConnectionByToolkit = new Map([
      ['gmail', { id: 'ca_1', toolkit: 'gmail', status: 'ACTIVE' }],
    ]);
    composioConnectionsByToolkitOverride = new Map([
      [
        'gmail',
        [
          { id: 'ca_1', toolkit: 'gmail', status: 'ACTIVE' },
          { id: 'ca_2', toolkit: 'gmail', status: 'ACTIVE' },
        ],
      ],
    ]);
    agentReadyState = { agentReady: new Set(['gmail']), loading: false, error: null };

    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });
    openAppsTab();

    const integrationsSection = screen.getByTestId('composio-integrations-card');
    expect(within(integrationsSection as HTMLElement).getByText('2')).toBeInTheDocument();
  });

  it('does not flood the integrations grid with Preview badges when the agent-ready RPC fails', () => {
    // CodeRabbit on #2361: when the agent-ready hook errors out
    // (loading=false, agentReady=empty, error set), we must NOT
    // label every curated toolkit as Preview — the UI has no
    // signal to draw that conclusion. Skills.tsx now falls back to
    // treating every toolkit as agent-ready in this state so the
    // page degrades to the pre-#2283 behaviour instead of
    // misrepresenting the agent surface.
    agentReadyState = { agentReady: new Set<string>(), loading: false, error: 'rpc unavailable' };

    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });
    openAppsTab();

    const integrationsSection = screen.getByTestId('composio-integrations-card');
    // No Preview badges anywhere in the integrations grid. The
    // badge carries a `data-testid` of the form
    // `composio-preview-badge-<slug>`; absence means we degraded
    // gracefully on RPC failure.
    const previewBadges = within(integrationsSection as HTMLElement).queryAllByTestId(
      /composio-preview-badge-/
    );
    expect(previewBadges).toHaveLength(0);
  });

  it('marks connected Zoho Mail as preview when the agent cannot use it yet', () => {
    composioToolkits = ['zoho_mail'];
    composioConnectionByToolkit = new Map([
      ['zoho_mail', { id: 'ca_zoho', toolkit: 'zoho_mail', status: 'ACTIVE' }],
    ]);
    agentReadyState = { agentReady: new Set<string>(['gmail']), loading: false, error: null };

    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });
    openAppsTab();

    const integrationsSection = screen.getByTestId('composio-integrations-card');
    const zohoTile = within(integrationsSection as HTMLElement).getByRole('button', {
      name: /Zoho Mail.*Preview/i,
    });

    expect(within(zohoTile).getByTestId('composio-preview-badge-zoho_mail')).toHaveTextContent(
      'Preview'
    );
    expect(within(zohoTile).getAllByText('Preview')).toHaveLength(2);

    fireEvent.click(zohoTile);

    expect(screen.getByText(/Agent integration coming soon/i)).toBeInTheDocument();
  });

  it('shows a local-mode composio API key banner when no key is configured', async () => {
    sessionToken = 'header.payload.local';
    composioModeStatus = { result: { mode: 'direct', api_key_set: false }, logs: [] };

    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });
    openAppsTab();

    await waitFor(() => {
      expect(screen.getByText(/No Composio API Key Configured/i)).toBeInTheDocument();
    });
    expect(screen.getByText(/Local mode uses your own Composio API key/i)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Open.*Settings/i })).toBeInTheDocument();
    expect(screen.queryByPlaceholderText('Search skills…')).not.toBeInTheDocument();
    expect(screen.queryByText('Gmail')).not.toBeInTheDocument();
  });
});
