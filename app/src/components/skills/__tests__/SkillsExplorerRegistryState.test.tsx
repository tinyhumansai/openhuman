import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  CatalogDetail,
  CatalogEntry,
  CatalogPage,
  CatalogPageQuery,
} from '../../../services/api/skillRegistryApi';
import SkillsPage from '../SkillsPage';

vi.mock('../../../services/api/skillsApi', () => ({
  skillsApi: {
    listWorkflows: vi.fn(),
    installWorkflowFromUrl: vi.fn(),
    uninstallWorkflow: vi.fn(),
  },
}));

vi.mock('../../../services/api/skillRegistryApi', async importOriginal => {
  const actual = await importOriginal<typeof import('../../../services/api/skillRegistryApi')>();
  return {
    ...actual,
    skillRegistryApi: {
      browsePage: vi.fn(),
      detail: vi.fn(),
      sources: vi.fn(),
      categories: vi.fn(),
      install: vi.fn(),
    },
  };
});

const ENTRY: CatalogEntry = {
  id: 'registry-skill-1',
  name: 'Registry Skill',
  description: 'A skill from the registry',
  source: 'built-in',
  category: 'productivity',
  author: 'Registry Author',
  version: '2.0.0',
  tags: ['registry'],
  platforms: [],
  download_url: 'https://example.com/SKILL.md',
  source_url: 'https://example.com/registry-skill-1',
  docs_path: null,
  commands: [],
  env_vars: [],
  license: 'MIT',
  installable: true,
};

const PORTAL_ENTRY: CatalogEntry = {
  ...ENTRY,
  id: 'lobehub/prompt-agent',
  name: 'Prompt Agent',
  source: 'LobeHub',
  download_url: '',
  source_url: 'https://lobehub.com/agent/prompt-agent',
  installable: false,
};

function pageOf(query: CatalogPageQuery, overrides: Partial<CatalogPage> = {}): CatalogPage {
  const entries = overrides.entries ?? [ENTRY];
  return {
    entries,
    total: entries.length,
    page: query.page,
    pageSize: query.pageSize,
    totalPages: 1,
    freshness: 'live',
    fetchedAt: null,
    refreshing: false,
    lastError: null,
    ...overrides,
  };
}

async function api() {
  const { skillsApi } = await import('../../../services/api/skillsApi');
  const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
  return { skillsApi, skillRegistryApi };
}

async function serve(overrides: Partial<CatalogPage> = {}) {
  const { skillRegistryApi } = await api();
  vi.mocked(skillRegistryApi.browsePage).mockImplementation(async query =>
    pageOf(query, overrides)
  );
}

function renderRegistry(onToast = vi.fn()) {
  render(
    <MemoryRouter>
      <SkillsPage initialTab="registry" onToast={onToast} />
    </MemoryRouter>
  );
  return onToast;
}

describe('SkillsExplorerTab registry state', () => {
  beforeEach(async () => {
    const { skillsApi, skillRegistryApi } = await api();
    vi.mocked(skillsApi.listWorkflows).mockReset().mockResolvedValue([]);
    vi.mocked(skillRegistryApi.browsePage).mockReset();
    vi.mocked(skillRegistryApi.install).mockReset();
    vi.mocked(skillRegistryApi.detail).mockReset().mockRejectedValue(new Error('no detail'));
    vi.mocked(skillRegistryApi.sources).mockReset().mockResolvedValue([]);
    await serve();
  });

  it('hints that the first catalog fetch can be slow while nothing is held yet', async () => {
    const { skillRegistryApi } = await api();
    let answer: (page: CatalogPage) => void = () => {};
    vi.mocked(skillRegistryApi.browsePage).mockImplementation(
      query =>
        new Promise(resolve => {
          answer = () => resolve(pageOf(query));
        })
    );

    renderRegistry();

    expect(await screen.findByTestId('registry-first-load')).toHaveTextContent(
      'Loading the skill catalog. The first fetch can take a minute.'
    );
    await act(async () => answer(pageOf({ page: 1, pageSize: 25 })));
    await screen.findByText('Registry Skill');
    expect(screen.queryByTestId('registry-first-load')).toBeNull();
  });

  it('shows the saved catalog with an offline banner whose Retry forces a refresh', async () => {
    const { skillRegistryApi } = await api();
    await serve({
      freshness: 'cached',
      fetchedAt: 1_700_000_000,
      lastError: { kind: 'unavailable', message: 'registry returned 503' },
    });

    renderRegistry();

    const banner = await screen.findByTestId('registry-offline');
    expect(banner).toHaveTextContent('Offline: showing the saved catalog from');
    expect(screen.getByText('Registry Skill')).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(screen.getByTestId('registry-retry'));
    });
    await waitFor(() => {
      expect(skillRegistryApi.browsePage).toHaveBeenLastCalledWith(
        expect.objectContaining({ forceRefresh: true })
      );
    });
  });

  it('says the saved catalog is shown while a refresh runs', async () => {
    await serve({ freshness: 'cached', refreshing: true, fetchedAt: 1_700_000_000 });

    renderRegistry();

    expect(await screen.findByTestId('registry-refreshing')).toHaveTextContent(
      'Showing the saved catalog while a fresh copy loads.'
    );
    expect(screen.queryByTestId('registry-offline')).toBeNull();
  });

  it('shows no registry banner for a live catalog', async () => {
    renderRegistry();

    await screen.findByText('Registry Skill');
    expect(screen.queryByTestId('registry-offline')).toBeNull();
    expect(screen.queryByTestId('registry-refreshing')).toBeNull();
    expect(screen.queryByTestId('registry-error')).toBeNull();
  });

  it('turns a throttled first fetch into a retry-after message', async () => {
    const { skillRegistryApi } = await api();
    vi.mocked(skillRegistryApi.browsePage).mockRejectedValue(
      new Error('SKILL_REGISTRY_RATE_LIMITED: rate limited by registry: retry after 42s')
    );

    renderRegistry();

    expect(await screen.findByTestId('registry-error')).toHaveTextContent(
      'The skill registry is busy. Try again in 42s.'
    );
  });

  it('turns an unreachable registry into a connection message', async () => {
    const { skillRegistryApi } = await api();
    vi.mocked(skillRegistryApi.browsePage).mockRejectedValueOnce(
      new Error('SKILL_REGISTRY_TIMEOUT: registry did not answer within 30s')
    );

    renderRegistry();

    expect(await screen.findByTestId('registry-error')).toHaveTextContent(
      'Could not reach the skill registry. Check your connection and try again.'
    );

    await serve();
    await act(async () => {
      fireEvent.click(screen.getByTestId('registry-retry'));
    });
    await screen.findByText('Registry Skill');
    expect(screen.queryByTestId('registry-error')).toBeNull();
  });

  it('links a non-installable entry to its source page', async () => {
    await serve({ entries: [PORTAL_ENTRY] });

    renderRegistry();

    const link = await screen.findByTestId('registry-source-link-lobehub/prompt-agent');
    expect(link).toHaveAttribute('href', 'https://lobehub.com/agent/prompt-agent');
    expect(link).toHaveTextContent('View source');
    expect(screen.queryByTestId('registry-install-lobehub/prompt-agent')).toBeNull();
  });

  it('trusts installable over a download URL', async () => {
    await serve({ entries: [{ ...ENTRY, installable: false }] });

    renderRegistry();

    expect(
      await screen.findByTestId('registry-not-installable-registry-skill-1')
    ).toBeInTheDocument();
    expect(screen.queryByTestId('registry-install-registry-skill-1')).toBeNull();
  });

  it('loads the entry detail and shows its overview', async () => {
    const { skillRegistryApi } = await api();
    const detail: CatalogDetail = {
      ...ENTRY,
      overview: 'Walks a pull request and leaves review notes.',
      install_identifier: 'registry-skill-1',
    };
    vi.mocked(skillRegistryApi.detail).mockResolvedValue(detail);

    renderRegistry();

    const target = await screen.findByTestId('registry-tile-registry-skill-1');
    await act(async () => {
      fireEvent.click(target);
    });

    expect(await screen.findByTestId('skill-detail-overview')).toHaveTextContent(
      'Walks a pull request and leaves review notes.'
    );
    expect(skillRegistryApi.detail).toHaveBeenCalledWith('registry-skill-1');
    expect(screen.queryByTestId('skill-detail-source-link')).toBeNull();
  });

  it('points the detail of a non-installable entry at its source page', async () => {
    await serve({ entries: [PORTAL_ENTRY] });

    renderRegistry();

    const tile = await screen.findByTestId('registry-tile-lobehub/prompt-agent');
    await act(async () => {
      fireEvent.click(tile);
    });

    expect(await screen.findByTestId('skill-detail-source-link')).toHaveAttribute(
      'href',
      'https://lobehub.com/agent/prompt-agent'
    );
  });

  it.each([
    [
      'SKILL_REGISTRY_NO_DIRECT_DOWNLOAD: no SKILL.md for registry-skill-1',
      'This entry has no SKILL.md to download. Open its source page to install it another way. https://example.com/registry-skill-1',
    ],
    [
      'SKILL_REGISTRY_UPSTREAM_AMBIGUOUS: registry-skill-1 has several authors',
      'More than one author publishes a skill with this name and the catalog does not say which one this is, so it cannot be installed automatically.',
    ],
    [
      'SKILL_REGISTRY_RATE_LIMITED: rate limited: retry after 7s',
      'The skill registry is busy. Try again in 7s.',
    ],
    ['SKILL_REGISTRY_RATE_LIMITED: rate limited', 'The skill registry is busy. Try again shortly.'],
    ['SKILL_REGISTRY_TOO_LARGE: SKILL.md exceeds 1 MiB', 'SKILL.md exceeds 1 MiB'],
  ])('maps install failure %s to a readable toast', async (raw, message) => {
    const { skillRegistryApi } = await api();
    vi.mocked(skillRegistryApi.install).mockRejectedValue(new Error(raw));

    const onToast = renderRegistry();

    const target = await screen.findByTestId('registry-install-registry-skill-1');
    await act(async () => {
      fireEvent.click(target);
    });

    await waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'error', message }));
    });
  });
});
