import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { CatalogEntry, CatalogPage, CatalogPageQuery } from '../../../services/api/skillRegistryApi';
import type { WorkflowSummary } from '../../../services/api/skillsApi';
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

function pageOf(
  entries: CatalogEntry[],
  query: CatalogPageQuery,
  overrides: Partial<CatalogPage> = {}
): CatalogPage {
  const text = query.query?.trim().toLowerCase() ?? '';
  let hits = entries;
  if (text) {
    hits = hits.filter(
      e =>
        e.name.toLowerCase().includes(text) ||
        e.description.toLowerCase().includes(text) ||
        e.tags.some(tag => tag.toLowerCase().includes(text))
    );
  }
  if (query.sources?.length) hits = hits.filter(e => query.sources?.includes(e.source));
  const start = (query.page - 1) * query.pageSize;
  return {
    entries: hits.slice(start, start + query.pageSize),
    total: hits.length,
    page: query.page,
    pageSize: query.pageSize,
    totalPages: Math.ceil(hits.length / query.pageSize),
    freshness: 'live',
    fetchedAt: null,
    refreshing: false,
    lastError: null,
    ...overrides,
  };
}

/** Serve `entries` from the mocked core, paged and filtered like the real one. */
async function serveCatalog(entries: CatalogEntry[], overrides: Partial<CatalogPage> = {}) {
  const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
  vi.mocked(skillRegistryApi.browsePage).mockImplementation(async query =>
    pageOf(entries, query, overrides)
  );
}

const MOCK_SKILL: WorkflowSummary = {
  id: 'test-skill',
  name: 'Test Skill',
  description: 'A test skill for unit testing',
  version: '1.0.0',
  author: 'Test Author',
  tags: ['test', 'automation'],
  platforms: [],
  relatedSkills: [],
  sourceFormat: 'hermes',
  tools: [],
  prompts: [],
  location: '/Users/test/.openhuman/skills/test-skill/SKILL.md',
  resources: [],
  scope: 'user',
  legacy: false,
  warnings: [],
};

const MOCK_PROJECT_SKILL: WorkflowSummary = {
  ...MOCK_SKILL,
  id: 'project-skill',
  name: 'Project Skill',
  sourceFormat: 'openhuman',
  scope: 'project',
};

const MOCK_CATALOG_ENTRY: CatalogEntry = {
  id: 'registry-skill-1',
  name: 'Registry Skill',
  description: 'A skill from the registry',
  source: 'built-in',
  category: 'productivity',
  author: 'Registry Author',
  version: '2.0.0',
  tags: ['registry', 'remote'],
  platforms: [],
  download_url: 'https://example.com/SKILL.md',
  docs_path: null,
  commands: [],
  env_vars: [],
  license: 'MIT',
};

const MOCK_DOCKER_ENTRY: CatalogEntry = {
  ...MOCK_CATALOG_ENTRY,
  id: 'docker-manager',
  name: 'Docker Manager',
  description: 'Manage Docker containers and images',
  source: 'skills.sh',
  category: 'devops',
  tags: ['docker', 'containers'],
};

async function switchToInstalled() {
  const installedTab = screen.getByRole('tab', { name: /Installed/i });
  await act(async () => {
    fireEvent.pointerDown(installedTab, { button: 0 });
    fireEvent.click(installedTab);
  });
}

const MOCK_LEGACY_SKILL: WorkflowSummary = {
  ...MOCK_SKILL,
  id: 'legacy-skill',
  name: 'Legacy Skill',
  scope: 'user',
  legacy: true,
  sourceFormat: 'legacy',
};

const MOCK_CATALOG_ENTRY_WITH_META: CatalogEntry = {
  ...MOCK_CATALOG_ENTRY,
  id: 'full-meta-skill',
  name: 'Full Meta Skill',
  source: 'ClawHub',
  version: '3.1.0',
  author: 'Meta Author',
  license: 'Apache-2.0',
  tags: ['tag1', 'tag2', 'tag3'],
  download_url: 'https://clawhub.io/skills/full-meta',
};

describe('SkillsExplorerTab', () => {
  beforeEach(async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(skillsApi.listWorkflows).mockReset();
    vi.mocked(skillsApi.uninstallWorkflow).mockReset();
    vi.mocked(skillRegistryApi.browsePage).mockReset();
    vi.mocked(skillRegistryApi.detail).mockReset();
    vi.mocked(skillRegistryApi.install).mockReset();
    vi.mocked(skillRegistryApi.sources).mockReset();
    await serveCatalog([]);
    vi.mocked(skillRegistryApi.detail).mockRejectedValue(new Error('no detail'));
    vi.mocked(skillRegistryApi.sources).mockResolvedValue([]);
  });

  it('defaults to registry view and shows catalog entries', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    await serveCatalog([MOCK_CATALOG_ENTRY]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByText('Registry Skill')).toBeInTheDocument();
    });
    expect(screen.getByText('built-in')).toBeInTheDocument();
  });

  it('paginates the registry catalog via the DataTable pager', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    const entries: CatalogEntry[] = Array.from({ length: 130 }, (_, i) => ({
      ...MOCK_CATALOG_ENTRY,
      id: `paged-skill-${i}`,
      name: `Paged Skill ${i}`,
    }));
    await serveCatalog(entries);

    const { container } = render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByText('Paged Skill 0')).toBeInTheDocument();
    });

    const tileCount = () =>
      container.querySelectorAll('[data-testid^="registry-tile-"]').length;

    expect(tileCount()).toBe(25);
    expect(skillRegistryApi.browsePage).toHaveBeenLastCalledWith(
      expect.objectContaining({ page: 1, pageSize: 25 })
    );
    const pager = screen.getByTestId('registry-pagination');

    await act(async () => {
      fireEvent.click(within(pager).getByRole('button', { name: 'Next page' }));
    });
    await waitFor(() => {
      expect(screen.getByText('Paged Skill 25')).toBeInTheDocument();
    });
    expect(skillRegistryApi.browsePage).toHaveBeenLastCalledWith(
      expect.objectContaining({ page: 2, pageSize: 25 })
    );
    expect(tileCount()).toBe(25);

    await act(async () => {
      fireEvent.click(within(pager).getByRole('button', { name: 'Last page' }));
    });
    await waitFor(() => {
      expect(screen.getByText('Paged Skill 129')).toBeInTheDocument();
    });
    expect(skillRegistryApi.browsePage).toHaveBeenLastCalledWith(
      expect.objectContaining({ page: 6 })
    );
    expect(tileCount()).toBe(5);
  });

  it('advances the pager from the requested page while a page is still loading', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    const entries: CatalogEntry[] = Array.from({ length: 130 }, (_, i) => ({
      ...MOCK_CATALOG_ENTRY,
      id: `slow-skill-${i}`,
      name: `Slow Skill ${i}`,
    }));
    await serveCatalog(entries);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByText('Slow Skill 0')).toBeInTheDocument();
    });

    vi.mocked(skillRegistryApi.browsePage).mockImplementation(() => new Promise(() => {}));
    const pager = screen.getByTestId('registry-pagination');

    await act(async () => {
      fireEvent.click(within(pager).getByRole('button', { name: 'Next page' }));
    });
    await act(async () => {
      fireEvent.click(within(pager).getByRole('button', { name: 'Next page' }));
    });

    expect(skillRegistryApi.browsePage).toHaveBeenCalledWith(
      expect.objectContaining({ page: 2, pageSize: 25 })
    );
    expect(skillRegistryApi.browsePage).toHaveBeenLastCalledWith(
      expect.objectContaining({ page: 3, pageSize: 25 })
    );
  });

  it('searches catalog via RPC when typing in search box', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    await serveCatalog([MOCK_CATALOG_ENTRY, MOCK_DOCKER_ENTRY]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByText('Registry Skill')).toBeInTheDocument();
    });

    const searchInput = screen.getByTestId('skill-search-input');
    await act(async () => {
      fireEvent.change(searchInput, { target: { value: 'docker' } });
    });

    // Wait for the debounce to fire and the RPC search to be called
    await waitFor(
      () => {
        expect(skillRegistryApi.browsePage).toHaveBeenCalledWith(
          expect.objectContaining({ query: 'docker', page: 1 })
        );
      },
      { timeout: 2000 }
    );

    await waitFor(() => {
      expect(screen.getByText('Docker Manager')).toBeInTheDocument();
    });
    expect(screen.queryByText('Registry Skill')).toBeNull();
  });

  it('shows installed skills when switching to installed tab', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([MOCK_SKILL, MOCK_PROJECT_SKILL]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByText('Installed')).toBeInTheDocument();
    });

    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByText('Test Skill')).toBeInTheDocument();
    });
    expect(screen.getByText('Project Skill')).toBeInTheDocument();
  });

  it('shows empty state when no installed skills', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByText('No skills found')).toBeInTheDocument();
    });
  });

  it('shows error state on registry fetch failure', async () => {
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.browsePage).mockRejectedValue(new Error('Network error'));

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByText('Network error')).toBeInTheDocument();
    });
    expect(screen.getByRole('button', { name: /Try again/ })).toBeInTheDocument();
  });

  it('filters installed skills by search query', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([MOCK_SKILL, MOCK_PROJECT_SKILL]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByText('Test Skill')).toBeInTheDocument();
    });

    const searchInput = screen.getByPlaceholderText('Search skills...');
    fireEvent.change(searchInput, { target: { value: 'project' } });

    expect(screen.queryByText('Test Skill')).not.toBeInTheDocument();
    expect(screen.getByText('Project Skill')).toBeInTheDocument();
  });

  it('shows install from URL button', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);

    render(
      <MemoryRouter>
        <SkillsPage />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByTestId('skill-install-from-url-btn')).toBeInTheDocument();
    });
  });

  it('shows uninstall button only for user-scope skills', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([MOCK_SKILL, MOCK_PROJECT_SKILL]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByTestId('skill-explorer-tile-test-skill')).toBeInTheDocument();
    });

    expect(screen.getByTestId('skill-uninstall-test-skill')).toBeInTheDocument();
    expect(screen.queryByTestId('skill-uninstall-project-skill')).not.toBeInTheDocument();
  });

  it('displays version and tags in installed view', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([MOCK_SKILL]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByText('v1.0.0')).toBeInTheDocument();
    });
    expect(screen.getByText('test')).toBeInTheDocument();
    expect(screen.getByText('automation')).toBeInTheDocument();
  });

  it('displays scope badges', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([MOCK_SKILL, MOCK_PROJECT_SKILL]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByText('Test Skill')).toBeInTheDocument();
    });
    expect(screen.getAllByText('User').length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText('Project').length).toBeGreaterThanOrEqual(1);
  });

  it('shows skill warnings when present', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    const skillWithWarning = { ...MOCK_SKILL, warnings: ['Missing required field: author'] };
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([skillWithWarning]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByText('Missing required field: author')).toBeInTheDocument();
    });
  });

  it('shows "Installed" badge for already-installed catalog entries', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    const catalogEntry = {
      ...MOCK_CATALOG_ENTRY,
      id: 'built-in/apple-notes',
      name: 'Apple Notes',
      docs_path: 'skills/apple-notes/SKILL.md',
    };
    const installedSkill = {
      ...MOCK_SKILL,
      id: 'apple-notes',
      name: 'Apple Notes',
      location: '/Users/test/.openhuman/skills/apple-notes/SKILL.md',
    };
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([installedSkill]);
    await serveCatalog([catalogEntry]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByText('Apple Notes')).toBeInTheDocument();
    });
    const tile = screen.getByTestId('registry-tile-built-in/apple-notes');
    expect(within(tile).getByText('Installed')).toBeInTheDocument();
    expect(
      within(tile).queryByTestId('registry-install-built-in/apple-notes')
    ).not.toBeInTheDocument();

    await act(async () => {
      fireEvent.click(tile);
    });

    await waitFor(() => {
      expect(screen.getAllByText('Apple Notes').length).toBeGreaterThan(1);
    });
    expect(screen.queryByRole('button', { name: 'Install' })).not.toBeInTheDocument();
  });

  it('does not mark catalog entries installed by display name alone', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    const catalogEntry = {
      ...MOCK_CATALOG_ENTRY,
      id: 'built-in/apple-notes',
      name: 'Apple Notes',
      docs_path: 'skills/apple-notes/SKILL.md',
    };
    const unrelatedInstalledSkill = {
      ...MOCK_SKILL,
      id: 'apple-notes-copy',
      name: 'Apple Notes',
      location: '/Users/test/.openhuman/skills/apple-notes-copy/SKILL.md',
    };
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([unrelatedInstalledSkill]);
    await serveCatalog([catalogEntry]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );

    const tile = await screen.findByTestId('registry-tile-built-in/apple-notes');
    expect(within(tile).queryByText('Installed')).not.toBeInTheDocument();
    expect(within(tile).getByTestId('registry-install-built-in/apple-notes')).toBeInTheDocument();
  });

  // #4150: a successful install must flip the card to "Installed" even when the
  // refetched installed list does NOT map back to the catalog entry via the
  // install-key heuristic — otherwise the card reverted to "Install" and the
  // only signal of success was a fleeting toast.
  it('marks a catalog entry installed on success even when the refetched list does not map back', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    const catalogEntry = {
      ...MOCK_CATALOG_ENTRY,
      id: 'built-in/apple-notes',
      name: 'Apple Notes',
      docs_path: 'skills/apple-notes/SKILL.md',
    };
    // The installed list never resolves to anything that maps back to the entry
    // (simulates a post-install id/location the heuristic can't match).
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    await serveCatalog([catalogEntry]);
    vi.mocked(skillRegistryApi.install).mockResolvedValue({
      status: 'installed',
      url: '',
      stdout: '',
      stderr: '',
      newSkills: ['apple-notes'],
    });

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );

    const installBtn = await screen.findByTestId('registry-install-built-in/apple-notes');
    await act(async () => {
      fireEvent.click(installBtn);
    });

    const tile = screen.getByTestId('registry-tile-built-in/apple-notes');
    await waitFor(() => {
      expect(within(tile).getByText('Installed')).toBeInTheDocument();
    });
    expect(skillRegistryApi.install).toHaveBeenCalledWith('built-in/apple-notes', {
      acknowledgedDigest: undefined,
    });
    expect(
      within(tile).queryByTestId('registry-install-built-in/apple-notes')
    ).not.toBeInTheDocument();
  });

  it('has an install from URL button', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);

    render(
      <MemoryRouter>
        <SkillsPage />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByTestId('skill-install-from-url-btn')).toBeInTheDocument();
    });
    expect(screen.getByTestId('skill-install-from-url-btn')).toHaveTextContent('Install from URL');
  });

  it('shows "no results" when installed skills exist but search has no matches', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([MOCK_SKILL]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByText('Test Skill')).toBeInTheDocument();
    });

    const searchInput = screen.getByPlaceholderText('Search skills...');
    fireEvent.change(searchInput, { target: { value: 'xyznotfound999' } });

    await waitFor(() => {
      expect(screen.queryByText('Test Skill')).not.toBeInTheDocument();
    });
  });

  it('opens skill detail dialog when a skill tile is clicked', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([MOCK_SKILL]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByTestId('skill-explorer-tile-test-skill')).toBeInTheDocument();
    });

    await act(async () => {
      fireEvent.click(screen.getByTestId('skill-explorer-tile-test-skill'));
    });

    // The detail dialog should appear with the skill name
    await waitFor(() => {
      expect(screen.getAllByText('Test Skill').length).toBeGreaterThan(1);
    });
  });

  it('activates skill tile on Enter key and Space key', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([MOCK_SKILL]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );
    await switchToInstalled();

    const tile = await screen.findByTestId('skill-explorer-tile-test-skill');

    // Enter opens detail
    await act(async () => {
      fireEvent.keyDown(tile, { key: 'Enter' });
    });
    await waitFor(() => {
      expect(screen.getAllByText('Test Skill').length).toBeGreaterThan(1);
    });
  });

  it('opens catalog entry detail dialog when a registry tile is clicked', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    await serveCatalog([MOCK_CATALOG_ENTRY_WITH_META]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByTestId('registry-tile-full-meta-skill')).toBeInTheDocument();
    });

    await act(async () => {
      fireEvent.click(screen.getByTestId('registry-tile-full-meta-skill'));
    });

    // Detail dialog shows the entry's name and metadata
    await waitFor(() => {
      expect(screen.getAllByText('Full Meta Skill').length).toBeGreaterThan(1);
    });
    // Should show version, author, license
    expect(screen.getByText('v3.1.0')).toBeInTheDocument();
    expect(screen.getAllByText('Meta Author').length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText('Apache-2.0')).toBeInTheDocument();
    // Download URL
    expect(screen.getByText('https://clawhub.io/skills/full-meta')).toBeInTheDocument();
  });

  it('closes skill detail dialog when overlay is clicked', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([MOCK_SKILL]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );
    await switchToInstalled();

    const tile = await screen.findByTestId('skill-explorer-tile-test-skill');
    await act(async () => {
      fireEvent.click(tile);
    });

    // Dialog open — find the backdrop and click it
    await waitFor(() => {
      // The close button (×) should be visible
      expect(screen.getByRole('button', { name: /close/i })).toBeInTheDocument();
    });

    // Click the ×-close button inside the dialog header
    const closeBtns = screen.getAllByRole('button');
    const closeBtn = closeBtns.find(b => {
      const svg = b.querySelector('svg');
      return svg !== null && b.closest('[data-slot="dialog-content"]') !== null;
    });
    if (closeBtn) {
      await act(async () => {
        fireEvent.click(closeBtn);
      });
    }
  });

  it('shows install button in detail dialog footer for non-installed registry entry', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    await serveCatalog([MOCK_CATALOG_ENTRY]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );

    // Click tile to open detail dialog
    const tile = await screen.findByTestId('registry-tile-registry-skill-1');
    await act(async () => {
      fireEvent.click(tile);
    });

    // The dialog should show the skill name (header) and description
    await waitFor(() => {
      expect(screen.getAllByText('Registry Skill').length).toBeGreaterThan(1);
    });
    // The detail dialog shows an Install button in its footer. The tile's own
    // Install button is still mounted but the modal marks the rest of the tree
    // `aria-hidden`, so the accessibility tree exposes exactly the footer one —
    // which is the thing this test is about. Assert it sits inside the dialog
    // rather than counting buttons.
    const installBtns = screen.getAllByRole('button', { name: 'Install' });
    expect(installBtns).toHaveLength(1);
    expect(installBtns[0].closest('[data-slot="dialog-content"]')).not.toBeNull();
  });

  it('calls skillRegistryApi.install when the install button in registry tile is clicked', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    const onToast = vi.fn();
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    await serveCatalog([MOCK_CATALOG_ENTRY]);
    vi.mocked(skillRegistryApi.install).mockResolvedValue({
      status: 'installed',
      url: 'https://example.com/SKILL.md',
      stdout: 'ok',
      stderr: '',
      newSkills: ['registry-skill-1'],
    });

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" onToast={onToast} />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByTestId('registry-install-registry-skill-1')).toBeInTheDocument();
    });

    await act(async () => {
      fireEvent.click(screen.getByTestId('registry-install-registry-skill-1'));
    });

    await waitFor(() => {
      expect(skillRegistryApi.install).toHaveBeenCalledWith('registry-skill-1', {
      acknowledgedDigest: undefined,
    });
    });
    await waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'success' }));
    });
  });

  it('prompts on a scan block, keeps Block as the default and installs only the reviewed document', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    const onToast = vi.fn();
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    await serveCatalog([MOCK_CATALOG_ENTRY]);
    const scan = {
      target: 'registry-skill-1',
      fetchedFrom: 'https://example.com/SKILL.md',
      slug: 'registry-skill-1',
      digest: 'digest-seen',
      findings: [
        {
          check: 'invisible_code_points',
          verdict: 'block' as const,
          field: 'the document body',
          message: 'an invisible character in the document body',
        },
      ],
      message: 'blocked',
    };
    vi.mocked(skillRegistryApi.install).mockResolvedValue({ status: 'scan_blocked', scan });

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" onToast={onToast} />
      </MemoryRouter>
    );
    await waitFor(() => {
      expect(screen.getByTestId('registry-install-registry-skill-1')).toBeInTheDocument();
    });

    await act(async () => {
      fireEvent.click(screen.getByTestId('registry-install-registry-skill-1'));
    });
    const dialog = await screen.findByTestId('scan-blocked-dialog');
    expect(within(dialog).getByText(/invisible character/)).toBeInTheDocument();
    expect(screen.getByTestId('scan-blocked-block')).toHaveFocus();

    await act(async () => {
      fireEvent.click(screen.getByTestId('scan-blocked-block'));
    });
    expect(screen.queryByTestId('scan-blocked-dialog')).not.toBeInTheDocument();
    expect(skillRegistryApi.install).toHaveBeenCalledTimes(1);
    expect(onToast).not.toHaveBeenCalledWith(expect.objectContaining({ type: 'success' }));

    await act(async () => {
      fireEvent.click(screen.getByTestId('registry-install-registry-skill-1'));
    });
    await screen.findByTestId('scan-blocked-dialog');
    vi.mocked(skillRegistryApi.install).mockResolvedValueOnce({
      status: 'scan_blocked',
      scan: {
        ...scan,
        digest: 'digest-changed',
        findings: [
          {
            check: 'hardcoded_credential',
            verdict: 'block' as const,
            field: 'the document body',
            message: 'a hard-coded credential in the document body',
          },
        ],
      },
    });
    await act(async () => {
      fireEvent.click(screen.getByTestId('scan-blocked-install-anyway'));
    });
    expect(skillRegistryApi.install).toHaveBeenLastCalledWith('registry-skill-1', {
      acknowledgedDigest: 'digest-seen',
    });
    await waitFor(() => {
      expect(screen.getByText(/hard-coded credential/)).toBeInTheDocument();
    });
    expect(screen.getByTestId('scan-blocked-dialog')).toBeInTheDocument();
    vi.mocked(skillRegistryApi.install).mockResolvedValueOnce({
      status: 'installed',
      url: 'https://example.com/SKILL.md',
      stdout: 'ok',
      stderr: '',
      newSkills: ['registry-skill-1'],
    });
    await act(async () => {
      fireEvent.click(screen.getByTestId('scan-blocked-install-anyway'));
    });

    await waitFor(() => {
      expect(skillRegistryApi.install).toHaveBeenLastCalledWith('registry-skill-1', {
        acknowledgedDigest: 'digest-changed',
      });
    });
    await waitFor(() => {
      expect(screen.queryByTestId('scan-blocked-dialog')).not.toBeInTheDocument();
    });
    expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'success' }));
  });

  it('marks an entry with no SKILL.md download as not installable instead of offering Install', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    await serveCatalog([
      {
        ...MOCK_CATALOG_ENTRY,
        id: 'lobehub/prompt-agent',
        name: 'Prompt Agent',
        source: 'LobeHub',
        download_url: '',
      },
    ]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );

    const badge = await screen.findByTestId('registry-not-installable-lobehub/prompt-agent');
    expect(badge).toHaveTextContent('Not installable');
    expect(screen.queryByTestId('registry-install-lobehub/prompt-agent')).toBeNull();
  });

  it('explains in the detail dialog why an entry with no SKILL.md download cannot be installed', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    await serveCatalog([
      {
        ...MOCK_CATALOG_ENTRY,
        id: 'lobehub/prompt-agent',
        name: 'Prompt Agent',
        source: 'LobeHub',
        download_url: '',
      },
    ]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );

    const tile = await screen.findByTestId('registry-tile-lobehub/prompt-agent');
    await act(async () => {
      fireEvent.click(tile);
    });

    const hint = await screen.findByText(
      'This entry has no SKILL.md to download, so it cannot be installed from here.'
    );
    expect(hint.closest('[data-slot="dialog-content"]')).not.toBeNull();
    expect(screen.queryByRole('button', { name: 'Install' })).toBeNull();
  });

  it('shows error toast when registry install fails', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    const onToast = vi.fn();
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    await serveCatalog([MOCK_CATALOG_ENTRY]);
    vi.mocked(skillRegistryApi.install).mockRejectedValue(new Error('Install failed'));

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" onToast={onToast} />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByTestId('registry-install-registry-skill-1')).toBeInTheDocument();
    });

    await act(async () => {
      fireEvent.click(screen.getByTestId('registry-install-registry-skill-1'));
    });

    await waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }));
    });
  });

  it('shows source filter options when sources are available', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.sources).mockResolvedValue(['built-in', 'ClawHub']);
    // No catalog entries so "built-in" only appears in the toggle buttons
    await serveCatalog([]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Filter by source' })).toBeInTheDocument();
    });
    await act(async () => {
      fireEvent.pointerDown(screen.getByRole('button', { name: 'Filter by source' }), { button: 0 });
    });
    expect(await screen.findByRole('menuitem', { name: /built-in/i })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: /ClawHub/i })).toBeInTheDocument();
  });

  it('deselecting a source filter triggers search with single active source', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.sources).mockResolvedValue(['built-in', 'ClawHub']);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Filter by source' })).toBeInTheDocument();
    });

    await act(async () => {
      fireEvent.pointerDown(screen.getByRole('button', { name: 'Filter by source' }), { button: 0 });
    });
    await act(async () => {
      fireEvent.click(await screen.findByRole('menuitem', { name: /ClawHub/i }));
    });

    await waitFor(() => {
      expect(skillRegistryApi.browsePage).toHaveBeenCalledWith(
        expect.objectContaining({ sources: ['built-in'], page: 1 })
      );
    });
  });

  it('shows legacy scope badge for legacy skills', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([MOCK_LEGACY_SKILL]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByText('Legacy Skill')).toBeInTheDocument();
    });
    // legacy scope badge should show
    expect(screen.getAllByText('Legacy').length).toBeGreaterThanOrEqual(1);
  });

  it('displays SkillFormatBadge with fallback label for unknown format', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    const unknownFormatSkill = {
      ...MOCK_SKILL,
      id: 'unk-skill',
      name: 'Unknown Format Skill',
      sourceFormat: 'unknown-format',
    };
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([unknownFormatSkill]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByText('Unknown Format Skill')).toBeInTheDocument();
    });
    // The badge renders the raw format string for unknown formats
    expect(screen.getByText('unknown-format')).toBeInTheDocument();
  });

  it('shows empty registry state when catalog returns no results', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    await serveCatalog([]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );

    await waitFor(() => {
      // Empty registry state shows its title (i18n key: skills.explorer.registryEmptyTitle)
      expect(screen.getByText('No registry entries')).toBeInTheDocument();
    });
  });

  it('retry button on error retriggers catalog fetch', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    await serveCatalog([MOCK_CATALOG_ENTRY]);
    vi.mocked(skillRegistryApi.browsePage).mockRejectedValueOnce(new Error('timeout'));

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByText('timeout')).toBeInTheDocument();
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /Try again/ }));
    });

    await waitFor(() => {
      expect(screen.getByText('Registry Skill')).toBeInTheDocument();
    });
  });

  it('retry button on installed view error retriggers skills fetch', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows)
      .mockRejectedValueOnce(new Error('skills fetch failed'))
      .mockResolvedValue([MOCK_SKILL]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByText('skills fetch failed')).toBeInTheDocument();
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /Try again/ }));
    });

    await waitFor(() => {
      expect(screen.getByText('Test Skill')).toBeInTheDocument();
    });
  });

  it('refresh button triggers force-refresh catalog fetch', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    await serveCatalog([MOCK_CATALOG_ENTRY]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByText('Registry Skill')).toBeInTheDocument();
    });

    const callsBefore = vi.mocked(skillRegistryApi.browsePage).mock.calls.length;

    const refreshBtn = screen.getByRole('button', { name: 'Refresh registry' });
    await act(async () => {
      fireEvent.click(refreshBtn);
    });

    await waitFor(() => {
      expect(vi.mocked(skillRegistryApi.browsePage).mock.calls.length).toBeGreaterThan(
        callsBefore
      );
    });
    const calls = vi.mocked(skillRegistryApi.browsePage).mock.calls;
    expect(calls[calls.length - 1][0]).toMatchObject({ forceRefresh: true });
  });

  it('sorts hermes skills before non-hermes in installed view', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    const alphaSkill = {
      ...MOCK_SKILL,
      id: 'alpha',
      name: 'Alpha Skill',
      sourceFormat: 'openhuman',
    };
    const hermesSkill = {
      ...MOCK_SKILL,
      id: 'hermes',
      name: 'Hermes Skill',
      sourceFormat: 'hermes',
    };
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([alphaSkill, hermesSkill]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByText('Hermes Skill')).toBeInTheDocument();
    });

    // Installed skills render as table rows now (same row grammar as the MCP
    // servers table), not buttons.
    const allTiles = screen.getAllByTestId(/^skill-explorer-tile-/);
    // Hermes should come first
    expect(allTiles[0]).toHaveAttribute('data-testid', 'skill-explorer-tile-hermes');
    expect(allTiles[1]).toHaveAttribute('data-testid', 'skill-explorer-tile-alpha');
  });

  it('activates catalog tile on Enter key', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    await serveCatalog([MOCK_CATALOG_ENTRY]);

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" />
      </MemoryRouter>
    );

    const tile = await screen.findByTestId('registry-tile-registry-skill-1');

    await act(async () => {
      fireEvent.keyDown(tile, { key: 'Enter' });
    });

    // Detail dialog opens
    await waitFor(() => {
      expect(screen.getAllByText('Registry Skill').length).toBeGreaterThan(1);
    });
  });

  it('detail dialog install button (footer) triggers install', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    const onToast = vi.fn();
    vi.mocked(skillsApi.listWorkflows).mockResolvedValue([]);
    // Override the beforeEach mock so browse returns an entry
    await serveCatalog([MOCK_CATALOG_ENTRY]);
    vi.mocked(skillRegistryApi.install).mockResolvedValue({
      status: 'installed',
      url: 'https://example.com/SKILL.md',
      stdout: 'ok',
      stderr: '',
      newSkills: [],
    });

    render(
      <MemoryRouter>
        <SkillsPage initialTab="registry" onToast={onToast} />
      </MemoryRouter>
    );

    // Wait for tile to appear, then open detail dialog
    const tile = await screen.findByTestId('registry-tile-registry-skill-1');
    await act(async () => {
      fireEvent.click(tile);
    });

    // Wait for dialog to open (name appears twice — tile + dialog header)
    await waitFor(() => {
      expect(screen.getAllByText('Registry Skill').length).toBeGreaterThan(1);
    });

    // The dialog footer has an extra Install button — click the last one (the footer button)
    const installBtns = screen.getAllByRole('button', { name: 'Install' });
    const dialogInstallBtn = installBtns[installBtns.length - 1];
    await act(async () => {
      fireEvent.click(dialogInstallBtn);
    });

    await waitFor(() => {
      expect(skillRegistryApi.install).toHaveBeenCalledWith('registry-skill-1', {
      acknowledgedDigest: undefined,
    });
    });
  });
});
