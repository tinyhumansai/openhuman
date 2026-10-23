/**
 * Tests for SearchPanel: the multi-provider web search settings.
 *
 * Covers the data-driven sections rendered from `config_get_search_settings`:
 *  - the header on/off switch and the page shell,
 *  - Providers tab: the Connected list (switch, detail line, status and
 *    deep-research badges, row menu actions) and the Add-a-provider catalogue
 *    (one-click via TinyHumans, the Connect dialog for keys and instance URLs),
 *  - the local-session state where managed routes are unavailable,
 *  - Routing tab: the roles table, its fallback-order dialog (reorder, remove,
 *    add, reset) and the Advanced presentation toggle,
 *  - Websites tab: the allowed-websites section (Allow all / Custom / Block all).
 */
import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import SearchPanel from './SearchPanel';

// ---------------------------------------------------------------------------
// Hoisted mocks
// ---------------------------------------------------------------------------
const hoisted = vi.hoisted(() => ({
  getSearchSettings: vi.fn(),
  updateSearchSettings: vi.fn(),
  localSession: false,
}));

vi.mock('../../../utils/tauriCommands/config', () => ({
  openhumanGetSearchSettings: (...a: unknown[]) => hoisted.getSearchSettings(...a),
  openhumanUpdateSearchSettings: (...a: unknown[]) => hoisted.updateSearchSettings(...a),
}));

// Identity translator so we can query by the stable i18n keys.
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

vi.mock('../../../utils/localSession', () => ({ isLocalSessionToken: () => hoisted.localSession }));

// ---------------------------------------------------------------------------
// Fixtures (shape of the core's config_get_search_settings response)
// ---------------------------------------------------------------------------
type Provider = Record<string, unknown> & { id: string };

function provider(id: string, overrides: Record<string, unknown> = {}): Provider {
  const base: Record<string, Provider> = {
    exa: {
      id: 'exa',
      label: 'Exa',
      enabled: true,
      route: 'managed',
      routes: ['managed', 'direct'],
      managed_available: true,
      key_configured: false,
      takes_key: true,
      usable: true,
      status: 'ready',
      roles: ['search', 'answer', 'contents'],
      docs_url: 'https://dashboard.exa.ai/api-keys',
    },
    gemini: {
      id: 'gemini',
      label: 'Gemini',
      enabled: true,
      route: 'managed',
      routes: ['managed', 'direct'],
      managed_available: true,
      key_configured: false,
      takes_key: true,
      usable: true,
      status: 'ready',
      roles: ['answer'],
      docs_url: 'https://aistudio.google.com/apikey',
      deep_research_available: false,
    },
    brave: {
      id: 'brave',
      label: 'Brave',
      enabled: false,
      route: 'direct',
      routes: ['direct'],
      managed_available: true,
      key_configured: false,
      takes_key: true,
      usable: false,
      status: 'disabled',
      roles: ['search'],
      docs_url: 'https://brave.com/search/api/',
    },
    tinyfish: {
      id: 'tinyfish',
      label: 'TinyFish',
      enabled: false,
      route: 'managed',
      routes: ['managed'],
      managed_available: true,
      key_configured: false,
      takes_key: false,
      usable: false,
      status: 'disabled',
      roles: ['search', 'contents'],
      docs_url: null,
    },
    searxng: {
      id: 'searxng',
      label: 'SearXNG',
      enabled: false,
      route: 'direct',
      routes: ['direct'],
      managed_available: true,
      key_configured: true,
      takes_key: false,
      usable: false,
      status: 'disabled',
      roles: ['search'],
      docs_url: 'https://docs.searxng.org/',
      base_url: 'http://localhost:8080',
    },
    keenable: {
      id: 'keenable',
      label: 'Keenable',
      enabled: false,
      route: 'direct',
      routes: ['direct'],
      managed_available: true,
      key_configured: false,
      takes_key: true,
      key_optional: true,
      usable: false,
      status: 'disabled',
      roles: ['search', 'contents'],
      docs_url: 'https://keenable.ai/console',
    },
  };
  return { ...base[id], ...overrides };
}

function settings(overrides: Record<string, unknown> = {}) {
  return {
    enabled: true,
    presentation: 'roles',
    presentation_provider: null,
    max_results: 5,
    timeout_secs: 15,
    managed_available: true,
    providers: [provider('exa'), provider('gemini'), provider('brave'), provider('searxng')],
    roles: { search: ['exa', 'brave', 'searxng'], answer: ['gemini', 'exa'], contents: ['exa'] },
    effective_roles: { search: ['exa'], answer: ['gemini', 'exa'], contents: ['exa'] },
    allowed_domains: ['reuters.com'],
    allow_all: false,
    ...overrides,
  };
}

const PLACEHOLDER = 'settings.search.allowedSitesPlaceholder';
const ALLOW_ALL = 'settings.search.accessAllowAll';
const CUSTOM = 'settings.search.accessCustom';
const BLOCK_ALL = 'settings.search.accessBlockAll';

const radio = (name: string) => screen.getByRole('radio', { name });
const row = (id: string) => screen.getByTestId(`search-provider-${id}`);
const tile = (id: string) => screen.getByTestId(`search-catalog-${id}`);
const roleRow = (role: string) => screen.getByTestId(`search-role-${role}`);

async function renderPanel(tab: 'providers' | 'routing' | 'websites' = 'providers') {
  renderWithProviders(<SearchPanel embedded />);
  await screen.findByTestId('search-providers');
  if (tab !== 'providers') fireEvent.click(screen.getByTestId(`search-tab-${tab}`));
}

/** Open a connected provider's row menu and pick an item by its label key. */
async function rowAction(id: string, labelKey: string) {
  const trigger = within(row(id)).getByRole('button', { name: 'settings.search.rowActions' });
  fireEvent.pointerDown(trigger, { button: 0, ctrlKey: false, pointerType: 'mouse' });
  fireEvent.click(await screen.findByRole('menuitem', { name: labelKey }));
}

/** Open a role's fallback-order dialog from the Routing table. */
async function openRole(role: string) {
  fireEvent.click(screen.getByTestId(`search-role-${role}-edit`));
  return screen.findByTestId(`search-role-${role}-dialog`);
}

beforeEach(() => {
  hoisted.localSession = false;
  hoisted.getSearchSettings.mockReset();
  hoisted.updateSearchSettings.mockReset();
  hoisted.getSearchSettings.mockResolvedValue({ result: settings() });
  hoisted.updateSearchSettings.mockResolvedValue({ result: settings() });
});

describe('SearchPanel — page and on/off', () => {
  test('renders as a titled page with chip tabs when not embedded', async () => {
    renderWithProviders(<SearchPanel />);
    expect(
      await screen.findByRole('heading', { name: 'settings.search.title' })
    ).toBeInTheDocument();
    expect(screen.getByTestId('search-tab-providers')).toBeInTheDocument();
    expect(screen.getByTestId('search-tab-routing')).toBeInTheDocument();
    expect(screen.getByTestId('search-tab-websites')).toBeInTheDocument();
  });

  test('the header switch persists enabled: false', async () => {
    await renderPanel();
    fireEvent.click(screen.getByTestId('search-enabled-toggle'));
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({ enabled: false })
    );
  });

  test('the returned settings replace local state; search off shows a notice', async () => {
    hoisted.updateSearchSettings.mockResolvedValue({ result: settings({ enabled: false }) });
    await renderPanel();
    expect(screen.queryByTestId('search-off-notice')).toBeNull();
    fireEvent.click(screen.getByTestId('search-enabled-toggle'));
    await screen.findByTestId('search-off-notice');
    expect(screen.getByTestId('search-enabled-toggle')).toHaveAttribute('aria-checked', 'false');
  });

  test('an RPC error shows on the status line and keeps the previous state', async () => {
    hoisted.updateSearchSettings.mockRejectedValue(new Error('boom'));
    await renderPanel();
    fireEvent.click(screen.getByTestId('search-enabled-toggle'));
    await screen.findByText('settings.search.statusError: boom');
    expect(screen.getByTestId('search-enabled-toggle')).toHaveAttribute('aria-checked', 'true');
  });

  test('a failed load shows the error', async () => {
    hoisted.getSearchSettings.mockRejectedValue(new Error('offline'));
    renderWithProviders(<SearchPanel embedded />);
    await screen.findByText('settings.search.statusError: offline');
  });
});

describe('SearchPanel — connected providers', () => {
  // #5136: the managed path must not read as an unattributed black box, so
  // every provider reached via TinyHumans is named on its own row.
  test('providers reached via TinyHumans are each named', async () => {
    await renderPanel();
    for (const id of ['exa', 'gemini']) {
      expect(row(id)).toHaveTextContent(id === 'exa' ? 'Exa' : 'Gemini');
      expect(screen.getByTestId(`search-provider-${id}-detail`)).toHaveTextContent(
        'settings.search.routeManaged'
      );
    }
  });

  test('lists only enabled providers, each with how it is reached', async () => {
    await renderPanel();
    expect(row('exa')).toBeInTheDocument();
    expect(row('gemini')).toBeInTheDocument();
    expect(screen.queryByTestId('search-provider-brave')).toBeNull();
    expect(within(row('exa')).getByTestId('search-provider-exa-detail')).toHaveTextContent(
      'settings.search.routeManaged'
    );
  });

  test('a ready provider shows no status badge; a problem does', async () => {
    hoisted.getSearchSettings.mockResolvedValue({
      result: settings({
        providers: [
          provider('exa', { route: 'direct', status: 'needs_key', usable: false }),
          provider('gemini'),
        ],
      }),
    });
    await renderPanel();
    expect(screen.queryByTestId('search-provider-gemini-status')).toBeNull();
    expect(screen.getByTestId('search-provider-exa-status')).toHaveTextContent(
      'settings.search.statusNeedsKey'
    );
    expect(screen.getByTestId('search-provider-exa-detail')).toHaveTextContent(
      'settings.search.detailNoKey'
    );
  });

  test('switching a connected provider off persists enabled: false', async () => {
    await renderPanel();
    fireEvent.click(screen.getByTestId('search-provider-gemini-toggle'));
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        providers: { gemini: { enabled: false } },
      })
    );
  });

  test('with nothing connected the list says so', async () => {
    hoisted.getSearchSettings.mockResolvedValue({
      result: settings({ providers: [provider('exa', { enabled: false }), provider('brave')] }),
    });
    await renderPanel();
    expect(screen.getByTestId('search-connected-empty')).toBeInTheDocument();
  });

  test('Gemini shows a deep-research badge once it is available', async () => {
    hoisted.getSearchSettings.mockResolvedValue({
      result: settings({
        providers: [provider('exa'), provider('gemini', { deep_research_available: true })],
      }),
    });
    await renderPanel();
    expect(screen.getByTestId('search-provider-gemini-deep-research')).toBeInTheDocument();
    expect(screen.queryByTestId('search-provider-exa-deep-research')).toBeNull();
  });

  test('"Use your own key" on a managed provider asks for a key and switches route', async () => {
    await renderPanel();
    await rowAction('exa', 'settings.search.actionUseOwnKey');
    const input = await screen.findByTestId('search-connect-exa-key');
    fireEvent.change(input, { target: { value: 'exa-secret' } });
    fireEvent.click(screen.getByTestId('search-connect-exa-submit'));
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        providers: { exa: { route: 'direct', api_key: 'exa-secret' } },
      })
    );
    await waitFor(() => expect(screen.queryByTestId('search-connect-exa-key')).toBeNull());
  });

  test('"Use your own key" with a stored key switches route without a dialog', async () => {
    hoisted.getSearchSettings.mockResolvedValue({
      result: settings({ providers: [provider('exa', { key_configured: true })] }),
    });
    await renderPanel();
    await rowAction('exa', 'settings.search.actionUseOwnKey');
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        providers: { exa: { route: 'direct' } },
      })
    );
  });

  test('a direct provider can go back via TinyHumans and drop its key', async () => {
    const direct = settings({
      providers: [provider('exa', { route: 'direct', key_configured: true })],
    });
    hoisted.getSearchSettings.mockResolvedValue({ result: direct });
    hoisted.updateSearchSettings.mockResolvedValue({ result: direct });
    await renderPanel();
    expect(screen.getByTestId('search-provider-exa-detail')).toHaveTextContent(
      'settings.search.detailOwnKey'
    );
    await rowAction('exa', 'settings.search.actionUseManaged');
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        providers: { exa: { route: 'managed' } },
      })
    );
    await screen.findByText('settings.search.statusSaved');
    await rowAction('exa', 'settings.search.actionRemoveKey');
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        providers: { exa: { api_key: '' } },
      })
    );
  });

  test('Gemini on the managed route offers a key for deep research without changing route', async () => {
    await renderPanel();
    await rowAction('gemini', 'settings.search.actionAddDeepResearchKey');
    const input = await screen.findByTestId('search-connect-gemini-key');
    expect(screen.getByText('settings.search.deepResearchHint')).toBeInTheDocument();
    fireEvent.change(input, { target: { value: '  g-key  ' } });
    fireEvent.click(screen.getByTestId('search-connect-gemini-submit'));
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        providers: { gemini: { api_key: 'g-key' } },
      })
    );
  });

  test('SearXNG shows its URL and can change it from the row menu', async () => {
    hoisted.getSearchSettings.mockResolvedValue({
      result: settings({ providers: [provider('searxng', { enabled: true })] }),
    });
    await renderPanel();
    expect(screen.getByTestId('search-provider-searxng-detail')).toHaveTextContent(
      'http://localhost:8080'
    );
    await rowAction('searxng', 'settings.search.actionEditUrl');
    const input = (await screen.findByTestId('search-connect-searxng-url')) as HTMLInputElement;
    expect(input.value).toBe('http://localhost:8080');
    expect(screen.queryByTestId('search-connect-searxng-key')).toBeNull();
    fireEvent.change(input, { target: { value: 'https://search.example.com ' } });
    fireEvent.click(screen.getByTestId('search-connect-searxng-submit'));
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        providers: { searxng: { base_url: 'https://search.example.com' } },
      })
    );
  });
});

describe('SearchPanel — wizard chrome', () => {
  test('Connected group cites the Routing tab normally', async () => {
    await renderPanel();
    expect(screen.getByTestId('search-providers')).toHaveTextContent(
      'settings.search.connectedDesc'
    );
  });

  // The copy points at a Routing tab the onboarding wizard hides.
  test('Connected group carries no description when the tab chrome is hidden', async () => {
    renderWithProviders(<SearchPanel embedded hideTabChrome />);
    await screen.findByTestId('search-providers');
    expect(screen.getByTestId('search-providers')).not.toHaveTextContent(
      'settings.search.connectedDesc'
    );
  });
});

describe('SearchPanel — add a provider', () => {
  const withTinyFish = () =>
    settings({
      providers: [
        provider('exa'),
        provider('gemini'),
        provider('tinyfish'),
        provider('brave'),
        provider('searxng'),
      ],
    });

  test('off providers are grouped by how they connect', async () => {
    hoisted.getSearchSettings.mockResolvedValue({ result: withTinyFish() });
    await renderPanel();
    const managed = screen.getByTestId('search-catalog-managed');
    const direct = screen.getByTestId('search-catalog-direct');
    expect(within(managed).getByTestId('search-catalog-tinyfish')).toBeInTheDocument();
    expect(within(direct).getByTestId('search-catalog-brave')).toBeInTheDocument();
    expect(within(direct).getByTestId('search-catalog-searxng')).toBeInTheDocument();
    expect(screen.queryByTestId('search-catalog-exa')).toBeNull();
    // Tiles say what each provider can do.
    expect(tile('tinyfish')).toHaveTextContent(
      'settings.search.roleSearch · settings.search.roleContents'
    );
  });

  test('a via-TinyHumans tile turns the provider on in one click', async () => {
    hoisted.getSearchSettings.mockResolvedValue({ result: withTinyFish() });
    await renderPanel();
    fireEvent.click(tile('tinyfish'));
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        providers: { tinyfish: { enabled: true } },
      })
    );
  });

  test('a provider that needs a key opens the Connect dialog first', async () => {
    await renderPanel();
    fireEvent.click(tile('brave'));
    await screen.findByTestId('search-connect-brave');
    const submit = screen.getByTestId('search-connect-brave-submit');
    expect(submit).toBeDisabled();
    expect(hoisted.updateSearchSettings).not.toHaveBeenCalled();
    // The dialog links to where a key is issued.
    expect(screen.getByText('settings.search.getApiKey').closest('a')).toHaveAttribute(
      'href',
      'https://brave.com/search/api/'
    );

    fireEvent.change(screen.getByTestId('search-connect-brave-key'), {
      target: { value: 'brave-key' },
    });
    fireEvent.click(submit);
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        providers: { brave: { enabled: true, api_key: 'brave-key' } },
      })
    );
    await waitFor(() => expect(screen.queryByTestId('search-connect-brave')).toBeNull());
  });

  test('a key-optional provider turns on in one click, without the Connect dialog', async () => {
    hoisted.getSearchSettings.mockResolvedValue({
      result: settings({ providers: [provider('exa'), provider('gemini'), provider('keenable')] }),
    });
    await renderPanel();
    const direct = screen.getByTestId('search-catalog-direct');
    expect(within(direct).getByTestId('search-catalog-keenable')).toHaveTextContent(
      'settings.search.detailKeyOptional'
    );
    fireEvent.click(tile('keenable'));
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        providers: { keenable: { enabled: true } },
      })
    );
    expect(screen.queryByTestId('search-connect-keenable')).toBeNull();
  });

  test('a connected key-optional provider needs no key but can take one', async () => {
    hoisted.getSearchSettings.mockResolvedValue({
      result: settings({
        providers: [
          provider('exa'),
          provider('gemini'),
          provider('keenable', { enabled: true, usable: true, status: 'ready' }),
        ],
      }),
    });
    await renderPanel();
    expect(screen.getByTestId('search-provider-keenable-detail')).toHaveTextContent(
      'settings.search.detailKeyOptional'
    );
    expect(screen.queryByTestId('search-provider-keenable-status')).toBeNull();

    await rowAction('keenable', 'settings.search.actionAddKey');
    await screen.findByTestId('search-connect-keenable');
    expect(screen.getByTestId('search-connect-keenable-key-optional')).toHaveTextContent(
      'settings.search.optionalKeyHint'
    );
    const submit = screen.getByTestId('search-connect-keenable-submit');
    expect(submit).toBeDisabled();
    fireEvent.change(screen.getByTestId('search-connect-keenable-key'), {
      target: { value: 'kn-key' },
    });
    fireEvent.click(submit);
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        providers: { keenable: { api_key: 'kn-key' } },
      })
    );
  });

  test('cancelling the Connect dialog saves nothing', async () => {
    await renderPanel();
    fireEvent.click(tile('brave'));
    await screen.findByTestId('search-connect-brave');
    fireEvent.click(screen.getByRole('button', { name: 'common.cancel' }));
    await waitFor(() => expect(screen.queryByTestId('search-connect-brave')).toBeNull());
    expect(hoisted.updateSearchSettings).not.toHaveBeenCalled();
  });

  test('a provider whose key is already stored turns on without a dialog', async () => {
    hoisted.getSearchSettings.mockResolvedValue({
      result: settings({
        providers: [provider('exa'), provider('brave', { key_configured: true })],
      }),
    });
    await renderPanel();
    fireEvent.click(tile('brave'));
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        providers: { brave: { enabled: true } },
      })
    );
    expect(screen.queryByTestId('search-connect-brave')).toBeNull();
  });

  test('SearXNG with an instance URL turns on directly; without one it asks', async () => {
    await renderPanel();
    fireEvent.click(tile('searxng'));
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        providers: { searxng: { enabled: true } },
      })
    );
  });

  test('SearXNG without an instance URL asks for one', async () => {
    hoisted.getSearchSettings.mockResolvedValue({
      result: settings({ providers: [provider('exa'), provider('searxng', { base_url: '' })] }),
    });
    await renderPanel();
    fireEvent.click(tile('searxng'));
    const input = await screen.findByTestId('search-connect-searxng-url');
    fireEvent.change(input, { target: { value: 'http://localhost:9000' } });
    fireEvent.click(screen.getByTestId('search-connect-searxng-submit'));
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        providers: { searxng: { enabled: true, base_url: 'http://localhost:9000' } },
      })
    );
  });
});

describe('SearchPanel — local session', () => {
  test('managed providers move to the own-key group and managed-only ones need sign-in', async () => {
    hoisted.localSession = true;
    hoisted.getSearchSettings.mockResolvedValue({
      result: settings({
        providers: [provider('exa', { enabled: false }), provider('tinyfish'), provider('brave')],
      }),
    });
    await renderPanel();
    expect(screen.getByText('settings.search.localManagedUnavailable')).toBeInTheDocument();
    expect(screen.queryByTestId('search-catalog-managed')).toBeNull();
    expect(tile('tinyfish')).toBeDisabled();
    expect(tile('tinyfish')).toHaveTextContent('settings.search.statusSignInRequired');

    // Exa can still be used with a key, so its tile opens the dialog.
    fireEvent.click(tile('exa'));
    fireEvent.change(await screen.findByTestId('search-connect-exa-key'), {
      target: { value: 'k' },
    });
    fireEvent.click(screen.getByTestId('search-connect-exa-submit'));
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        providers: { exa: { enabled: true, route: 'direct', api_key: 'k' } },
      })
    );
  });

  test('a signed-in session shows no local-session hint', async () => {
    await renderPanel();
    expect(screen.queryByText('settings.search.localManagedUnavailable')).toBeNull();
  });
});

describe('SearchPanel — routing', () => {
  test('each role shows the provider serving it and its fallbacks', async () => {
    await renderPanel('routing');
    expect(within(roleRow('search')).getByTestId('search-role-search-serving')).toHaveTextContent(
      'Exa'
    );
    expect(roleRow('search')).toHaveTextContent('settings.search.roleNoFallback');
    expect(within(roleRow('answer')).getByTestId('search-role-answer-serving')).toHaveTextContent(
      'Gemini'
    );
    expect(roleRow('answer')).toHaveTextContent('settings.search.roleFallbacks');
  });

  test('a role with no usable provider says so', async () => {
    hoisted.getSearchSettings.mockResolvedValue({
      result: settings({ effective_roles: { search: ['exa'], answer: [], contents: ['exa'] } }),
    });
    await renderPanel('routing');
    expect(screen.getByTestId('search-role-answer-serving')).toHaveTextContent(
      'settings.search.roleNoProviderShort'
    );
  });

  test('the dialog lists the order and marks the provider in use', async () => {
    await renderPanel('routing');
    const dialog = await openRole('search');
    const items = within(dialog).getAllByTestId(/^search-role-search-provider-/);
    expect(items.map(i => i.dataset.testid)).toEqual([
      'search-role-search-provider-exa',
      'search-role-search-provider-brave',
      'search-role-search-provider-searxng',
    ]);
    expect(within(dialog).getByTestId('search-role-search-provider-exa')).toHaveAttribute(
      'data-serving',
      'true'
    );
    expect(
      within(within(dialog).getByTestId('search-role-search-provider-brave')).getByText(
        'settings.search.roleUnavailable'
      )
    ).toBeInTheDocument();
  });

  test('moving a provider down saves the new order', async () => {
    await renderPanel('routing');
    const dialog = await openRole('search');
    fireEvent.click(
      within(dialog).getAllByRole('button', { name: 'settings.search.roleMoveDown' })[0]
    );
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        roles: { search: ['brave', 'exa', 'searxng'] },
      })
    );
  });

  test('the first provider cannot move up and the last cannot move down', async () => {
    await renderPanel('routing');
    const dialog = await openRole('search');
    const ups = within(dialog).getAllByRole('button', { name: 'settings.search.roleMoveUp' });
    const downs = within(dialog).getAllByRole('button', { name: 'settings.search.roleMoveDown' });
    expect(ups[0]).toBeDisabled();
    expect(downs[downs.length - 1]).toBeDisabled();
    expect(ups[1]).not.toBeDisabled();
  });

  test('removing a fallback saves the shorter order; the last one cannot be removed', async () => {
    await renderPanel('routing');
    const dialog = await openRole('search');
    fireEvent.click(
      within(dialog).getAllByRole('button', { name: 'settings.search.roleRemove' })[1]
    );
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        roles: { search: ['exa', 'searxng'] },
      })
    );
    // The footer button, not the header's icon-only close.
    fireEvent.click(screen.getByText('common.close', { selector: 'button' }));
    const contents = await openRole('contents');
    expect(
      within(contents).getByRole('button', { name: 'settings.search.roleRemove' })
    ).toBeDisabled();
  });

  test('a provider not in the order can be added back to the end', async () => {
    hoisted.getSearchSettings.mockResolvedValue({
      result: settings({
        roles: { search: ['exa', 'brave'], answer: ['gemini', 'exa'], contents: ['exa'] },
      }),
    });
    await renderPanel('routing');
    const dialog = await openRole('search');
    fireEvent.click(within(dialog).getByTestId('search-role-search-add-searxng'));
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        roles: { search: ['exa', 'brave', 'searxng'] },
      })
    );
  });

  test('reset sends an empty order to restore the default', async () => {
    await renderPanel('routing');
    await openRole('answer');
    fireEvent.click(screen.getByTestId('search-role-answer-reset'));
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({ roles: { answer: [] } })
    );
  });
});

describe('SearchPanel — advanced', () => {
  test('exposing provider tools switches presentation to all_tools', async () => {
    await renderPanel('routing');
    fireEvent.click(screen.getByTestId('search-presentation-toggle'));
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({ presentation: 'all_tools' })
    );
  });

  test('turning it off restores role presentation', async () => {
    hoisted.getSearchSettings.mockResolvedValue({
      result: settings({ presentation: 'all_tools' }),
    });
    await renderPanel('routing');
    fireEvent.click(screen.getByTestId('search-presentation-toggle'));
    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({ presentation: 'roles' })
    );
  });
});

describe('SearchPanel — allowed websites', () => {
  test('explicit host list → starts in Custom mode with the editor populated', async () => {
    await renderPanel('websites');
    await waitFor(() => {
      const ta = screen.getByPlaceholderText(PLACEHOLDER) as HTMLTextAreaElement;
      expect(ta.value).toBe('reuters.com');
    });
    expect(radio(CUSTOM)).toHaveAttribute('aria-checked', 'true');
    expect(radio(ALLOW_ALL)).toHaveAttribute('aria-checked', 'false');
  });

  test('selecting "Allow all" persists allow_all: true and hides the editor', async () => {
    await renderPanel('websites');
    await screen.findByPlaceholderText(PLACEHOLDER);

    fireEvent.click(radio(ALLOW_ALL));

    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({ allow_all: true })
    );
    expect(screen.queryByPlaceholderText(PLACEHOLDER)).toBeNull();
  });

  test('selecting "Block all" persists an empty allowlist and hides the editor', async () => {
    await renderPanel('websites');
    await screen.findByPlaceholderText(PLACEHOLDER);

    fireEvent.click(radio(BLOCK_ALL));

    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        allowed_domains: [],
        allow_all: false,
      })
    );
    expect(screen.queryByPlaceholderText(PLACEHOLDER)).toBeNull();
  });

  test('Custom: saving an edited host list persists allowed_domains + allow_all: false', async () => {
    await renderPanel('websites');
    const textarea = await screen.findByPlaceholderText(PLACEHOLDER);

    fireEvent.change(textarea, { target: { value: 'github.com\n  apnews.com  \n\n' } });
    fireEvent.click(screen.getByText('settings.search.allowedSitesSave'));

    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        allowed_domains: ['github.com', 'apnews.com'],
        allow_all: false,
      })
    );
  });

  test('Custom: pasted URLs are normalized to bare hosts before persisting', async () => {
    await renderPanel('websites');
    const textarea = await screen.findByPlaceholderText(PLACEHOLDER);

    fireEvent.change(textarea, {
      target: { value: 'https://reuters.com/markets\nhttp://apnews.com/\ngithub.com' },
    });
    fireEvent.click(screen.getByText('settings.search.allowedSitesSave'));

    await waitFor(() =>
      expect(hoisted.updateSearchSettings).toHaveBeenCalledWith({
        allowed_domains: ['reuters.com', 'apnews.com', 'github.com'],
        allow_all: false,
      })
    );
  });

  test('allow_all settings → starts in Allow-all mode with no editor', async () => {
    hoisted.getSearchSettings.mockResolvedValue({
      result: settings({ allowed_domains: ['*'], allow_all: true }),
    });
    await renderPanel('websites');

    await waitFor(() => expect(radio(ALLOW_ALL)).toHaveAttribute('aria-checked', 'true'));
    expect(screen.queryByPlaceholderText(PLACEHOLDER)).toBeNull();
  });

  test('empty allowlist → starts in Block-all mode with no editor', async () => {
    hoisted.getSearchSettings.mockResolvedValue({
      result: settings({ allowed_domains: [], allow_all: false }),
    });
    await renderPanel('websites');

    await waitFor(() => expect(radio(BLOCK_ALL)).toHaveAttribute('aria-checked', 'true'));
    expect(screen.queryByPlaceholderText(PLACEHOLDER)).toBeNull();
  });

  test('switching Block → Custom keeps the previously typed hosts', async () => {
    await renderPanel('websites');
    const textarea = (await screen.findByPlaceholderText(PLACEHOLDER)) as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: 'example.com' } });

    fireEvent.click(radio(BLOCK_ALL));
    await waitFor(() => expect(screen.queryByPlaceholderText(PLACEHOLDER)).toBeNull());
    await waitFor(() => expect(hoisted.updateSearchSettings).toHaveBeenCalled());
    await screen.findByText('settings.search.statusSaved');
    fireEvent.click(radio(CUSTOM));

    const reopened = (await screen.findByPlaceholderText(PLACEHOLDER)) as HTMLTextAreaElement;
    expect(reopened.value).toBe('example.com');
  });
});
