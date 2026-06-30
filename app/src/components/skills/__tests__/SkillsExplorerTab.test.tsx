import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { CatalogEntry } from '../../../services/api/skillRegistryApi';
import type { WorkflowSummary } from '../../../services/api/workflowsApi';
import SkillsExplorerTab from '../SkillsExplorerTab';

vi.mock('../../../services/api/workflowsApi', () => ({
  workflowsApi: {
    listWorkflows: vi.fn(),
    installWorkflowFromUrl: vi.fn(),
    uninstallWorkflow: vi.fn(),
  },
}));

vi.mock('../../../services/api/skillRegistryApi', () => ({
  skillRegistryApi: {
    browse: vi.fn(),
    search: vi.fn(),
    sources: vi.fn(),
    categories: vi.fn(),
    install: vi.fn(),
  },
}));

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
  const installedTab = screen.getByText('Installed', { selector: 'button' });
  await act(async () => {
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
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(workflowsApi.listWorkflows).mockReset();
    vi.mocked(workflowsApi.uninstallWorkflow).mockReset();
    vi.mocked(skillRegistryApi.browse).mockReset();
    vi.mocked(skillRegistryApi.search).mockReset();
    vi.mocked(skillRegistryApi.install).mockReset();
    vi.mocked(skillRegistryApi.sources).mockReset();
    vi.mocked(skillRegistryApi.browse).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.search).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.sources).mockResolvedValue([]);
  });

  it('defaults to registry view and shows catalog entries', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.browse).mockResolvedValue([MOCK_CATALOG_ENTRY]);

    render(<SkillsExplorerTab />);

    await waitFor(() => {
      expect(screen.getByText('Registry Skill')).toBeInTheDocument();
    });
    expect(screen.getByText('built-in')).toBeInTheDocument();
  });

  it('paginates the registry catalog via the Show more control', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    const entries: CatalogEntry[] = Array.from({ length: 130 }, (_, i) => ({
      ...MOCK_CATALOG_ENTRY,
      id: `paged-skill-${i}`,
      name: `Paged Skill ${i}`,
    }));
    vi.mocked(skillRegistryApi.browse).mockResolvedValue(entries);

    const { container } = render(<SkillsExplorerTab />);

    await waitFor(() => {
      expect(screen.getByText('Paged Skill 0')).toBeInTheDocument();
    });

    const tileCount = () =>
      container.querySelectorAll('[data-testid^="registry-tile-"]').length;

    // First page only: 60 of 130 revealed.
    expect(tileCount()).toBe(60);

    await act(async () => {
      fireEvent.click(screen.getByTestId('registry-show-more'));
    });
    expect(tileCount()).toBe(120);

    await act(async () => {
      fireEvent.click(screen.getByTestId('registry-show-more'));
    });
    // All 130 revealed → the control disappears.
    expect(tileCount()).toBe(130);
    expect(screen.queryByTestId('registry-show-more')).toBeNull();
  });

  it('searches catalog via RPC when typing in search box', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.browse).mockResolvedValue([MOCK_CATALOG_ENTRY]);
    vi.mocked(skillRegistryApi.search).mockResolvedValue([MOCK_DOCKER_ENTRY]);

    render(<SkillsExplorerTab />);

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
        expect(skillRegistryApi.search).toHaveBeenCalledWith('docker', undefined);
      },
      { timeout: 2000 }
    );

    await waitFor(() => {
      expect(screen.getByText('Docker Manager')).toBeInTheDocument();
    });
  });

  it('shows installed skills when switching to installed tab', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([MOCK_SKILL, MOCK_PROJECT_SKILL]);

    render(<SkillsExplorerTab />);

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
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);

    render(<SkillsExplorerTab />);
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByText('No skills found')).toBeInTheDocument();
    });
  });

  it('shows error state on registry fetch failure', async () => {
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.browse).mockRejectedValue(new Error('Network error'));

    render(<SkillsExplorerTab />);

    await waitFor(() => {
      expect(screen.getByText('Network error')).toBeInTheDocument();
    });
    expect(screen.getByRole('button', { name: /Try again/ })).toBeInTheDocument();
  });

  it('filters installed skills by search query', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([MOCK_SKILL, MOCK_PROJECT_SKILL]);

    render(<SkillsExplorerTab />);
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
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);

    render(<SkillsExplorerTab />);

    await waitFor(() => {
      expect(screen.getByTestId('skill-install-from-url-btn')).toBeInTheDocument();
    });
  });

  it('shows uninstall button only for user-scope skills', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([MOCK_SKILL, MOCK_PROJECT_SKILL]);

    render(<SkillsExplorerTab />);
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByTestId('skill-explorer-tile-test-skill')).toBeInTheDocument();
    });

    expect(screen.getByTestId('skill-uninstall-test-skill')).toBeInTheDocument();
    expect(screen.queryByTestId('skill-uninstall-project-skill')).not.toBeInTheDocument();
  });

  it('displays version and tags in installed view', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([MOCK_SKILL]);

    render(<SkillsExplorerTab />);
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByText('v1.0.0')).toBeInTheDocument();
    });
    expect(screen.getByText('test')).toBeInTheDocument();
    expect(screen.getByText('automation')).toBeInTheDocument();
  });

  it('displays scope badges', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([MOCK_SKILL, MOCK_PROJECT_SKILL]);

    render(<SkillsExplorerTab />);
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByText('Test Skill')).toBeInTheDocument();
    });
    expect(screen.getAllByText('User').length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText('Project').length).toBeGreaterThanOrEqual(1);
  });

  it('shows skill warnings when present', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const skillWithWarning = { ...MOCK_SKILL, warnings: ['Missing required field: author'] };
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([skillWithWarning]);

    render(<SkillsExplorerTab />);
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByText('Missing required field: author')).toBeInTheDocument();
    });
  });

  it('shows "Installed" badge for already-installed catalog entries', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
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
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([installedSkill]);
    vi.mocked(skillRegistryApi.browse).mockResolvedValue([catalogEntry]);

    render(<SkillsExplorerTab />);

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
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
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
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([unrelatedInstalledSkill]);
    vi.mocked(skillRegistryApi.browse).mockResolvedValue([catalogEntry]);

    render(<SkillsExplorerTab />);

    const tile = await screen.findByTestId('registry-tile-built-in/apple-notes');
    expect(within(tile).queryByText('Installed')).not.toBeInTheDocument();
    expect(within(tile).getByTestId('registry-install-built-in/apple-notes')).toBeInTheDocument();
  });

  // #4150: a successful install must flip the card to "Installed" even when the
  // refetched installed list does NOT map back to the catalog entry via the
  // install-key heuristic — otherwise the card reverted to "Install" and the
  // only signal of success was a fleeting toast.
  it('marks a catalog entry installed on success even when the refetched list does not map back', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    const catalogEntry = {
      ...MOCK_CATALOG_ENTRY,
      id: 'built-in/apple-notes',
      name: 'Apple Notes',
      docs_path: 'skills/apple-notes/SKILL.md',
    };
    // The installed list never resolves to anything that maps back to the entry
    // (simulates a post-install id/location the heuristic can't match).
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.browse).mockResolvedValue([catalogEntry]);
    vi.mocked(skillRegistryApi.install).mockResolvedValue({
      url: '',
      stdout: '',
      stderr: '',
      newSkills: ['apple-notes'],
    });

    render(<SkillsExplorerTab />);

    const installBtn = await screen.findByTestId('registry-install-built-in/apple-notes');
    await act(async () => {
      fireEvent.click(installBtn);
    });

    const tile = screen.getByTestId('registry-tile-built-in/apple-notes');
    await waitFor(() => {
      expect(within(tile).getByText('Installed')).toBeInTheDocument();
    });
    expect(skillRegistryApi.install).toHaveBeenCalledWith('built-in/apple-notes');
    expect(
      within(tile).queryByTestId('registry-install-built-in/apple-notes')
    ).not.toBeInTheDocument();
  });

  it('has an install from URL button', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);

    render(<SkillsExplorerTab />);

    await waitFor(() => {
      expect(screen.getByTestId('skill-install-from-url-btn')).toBeInTheDocument();
    });
    expect(screen.getByTestId('skill-install-from-url-btn')).toHaveTextContent('Install from URL');
  });

  it('shows "no results" when installed skills exist but search has no matches', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([MOCK_SKILL]);

    render(<SkillsExplorerTab />);
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
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([MOCK_SKILL]);

    render(<SkillsExplorerTab />);
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
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([MOCK_SKILL]);

    render(<SkillsExplorerTab />);
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
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.browse).mockResolvedValue([MOCK_CATALOG_ENTRY_WITH_META]);

    render(<SkillsExplorerTab />);

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
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([MOCK_SKILL]);

    render(<SkillsExplorerTab />);
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
      return svg !== null && b.closest('[class*="fixed inset-0"]') !== null;
    });
    if (closeBtn) {
      await act(async () => {
        fireEvent.click(closeBtn);
      });
    }
  });

  it('shows install button in detail dialog footer for non-installed registry entry', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.browse).mockResolvedValue([MOCK_CATALOG_ENTRY]);

    render(<SkillsExplorerTab />);

    // Click tile to open detail dialog
    const tile = await screen.findByTestId('registry-tile-registry-skill-1');
    await act(async () => {
      fireEvent.click(tile);
    });

    // The dialog should show the skill name (header) and description
    await waitFor(() => {
      expect(screen.getAllByText('Registry Skill').length).toBeGreaterThan(1);
    });
    // The detail dialog shows a second install button in the footer (in addition to the tile's button)
    const installBtns = screen.getAllByRole('button', { name: 'Install' });
    expect(installBtns.length).toBeGreaterThanOrEqual(2);
  });

  it('calls skillRegistryApi.install when the install button in registry tile is clicked', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    const onToast = vi.fn();
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.browse).mockResolvedValue([MOCK_CATALOG_ENTRY]);
    vi.mocked(skillRegistryApi.install).mockResolvedValue({
      url: 'https://example.com/SKILL.md',
      stdout: 'ok',
      stderr: '',
      newSkills: ['registry-skill-1'],
    });

    render(<SkillsExplorerTab onToast={onToast} />);

    await waitFor(() => {
      expect(screen.getByTestId('registry-install-registry-skill-1')).toBeInTheDocument();
    });

    await act(async () => {
      fireEvent.click(screen.getByTestId('registry-install-registry-skill-1'));
    });

    await waitFor(() => {
      expect(skillRegistryApi.install).toHaveBeenCalledWith('registry-skill-1');
    });
    await waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'success' }));
    });
  });

  it('shows error toast when registry install fails', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    const onToast = vi.fn();
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.browse).mockResolvedValue([MOCK_CATALOG_ENTRY]);
    vi.mocked(skillRegistryApi.install).mockRejectedValue(new Error('Install failed'));

    render(<SkillsExplorerTab onToast={onToast} />);

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

  it('shows source toggle buttons when sources are available', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.sources).mockResolvedValue(['built-in', 'ClawHub']);
    // No catalog entries so "built-in" only appears in the toggle buttons
    vi.mocked(skillRegistryApi.browse).mockResolvedValue([]);

    render(<SkillsExplorerTab />);

    // Source toggles are rendered as buttons with their source name text
    await waitFor(() => {
      const buttons = screen.getAllByRole('button');
      const sourceButtons = buttons.filter(
        b => b.textContent === 'built-in' || b.textContent === 'ClawHub'
      );
      expect(sourceButtons.length).toBe(2);
    });
  });

  it('deselecting a source filter triggers search with single active source', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.sources).mockResolvedValue(['built-in', 'ClawHub']);
    vi.mocked(skillRegistryApi.browse).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.search).mockResolvedValue([]);

    render(<SkillsExplorerTab />);

    // Wait for sources to load and buttons to appear
    await waitFor(() => {
      const buttons = screen.getAllByRole('button');
      expect(buttons.some(b => b.textContent === 'ClawHub')).toBe(true);
    });

    // Deselect 'ClawHub' — only 'built-in' remains active → triggers search with source filter
    const clawhubBtn = screen.getAllByRole('button').find(b => b.textContent === 'ClawHub')!;
    await act(async () => {
      fireEvent.click(clawhubBtn);
    });

    await waitFor(() => {
      expect(skillRegistryApi.search).toHaveBeenCalledWith('', 'built-in');
    });
  });

  it('shows catalog count in Registry tab badge when entries exist', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.browse).mockResolvedValue([MOCK_CATALOG_ENTRY, MOCK_DOCKER_ENTRY]);

    render(<SkillsExplorerTab />);

    await waitFor(() => {
      // The catalog count badge (2) should appear in the Registry tab
      expect(screen.getByText('2')).toBeInTheDocument();
    });
  });

  it('shows installed skill count in Installed tab badge', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([MOCK_SKILL, MOCK_PROJECT_SKILL]);

    render(<SkillsExplorerTab />);

    await waitFor(() => {
      // The skills count badge (2) should appear in the Installed tab
      expect(screen.getByText('2')).toBeInTheDocument();
    });
  });

  it('shows legacy scope badge for legacy skills', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([MOCK_LEGACY_SKILL]);

    render(<SkillsExplorerTab />);
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByText('Legacy Skill')).toBeInTheDocument();
    });
    // legacy scope badge should show
    expect(screen.getAllByText('Legacy').length).toBeGreaterThanOrEqual(1);
  });

  it('displays SkillFormatBadge with fallback label for unknown format', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const unknownFormatSkill = {
      ...MOCK_SKILL,
      id: 'unk-skill',
      name: 'Unknown Format Skill',
      sourceFormat: 'unknown-format',
    };
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([unknownFormatSkill]);

    render(<SkillsExplorerTab />);
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByText('Unknown Format Skill')).toBeInTheDocument();
    });
    // The badge renders the raw format string for unknown formats
    expect(screen.getByText('unknown-format')).toBeInTheDocument();
  });

  it('shows empty registry state when catalog returns no results', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.browse).mockResolvedValue([]);

    render(<SkillsExplorerTab />);

    await waitFor(() => {
      // Empty registry state shows its title (i18n key: skills.explorer.registryEmptyTitle)
      expect(screen.getByText('No registry entries')).toBeInTheDocument();
    });
  });

  it('retry button on error retriggers catalog fetch', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.browse)
      .mockRejectedValueOnce(new Error('timeout'))
      .mockResolvedValue([MOCK_CATALOG_ENTRY]);

    render(<SkillsExplorerTab />);

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
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    vi.mocked(workflowsApi.listWorkflows)
      .mockRejectedValueOnce(new Error('skills fetch failed'))
      .mockResolvedValue([MOCK_SKILL]);

    render(<SkillsExplorerTab />);
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
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.browse).mockResolvedValue([MOCK_CATALOG_ENTRY]);

    render(<SkillsExplorerTab />);

    await waitFor(() => {
      expect(screen.getByText('Registry Skill')).toBeInTheDocument();
    });

    const callsBefore = vi.mocked(skillRegistryApi.browse).mock.calls.length;

    const refreshBtn = screen.getByTitle('Refresh registry');
    await act(async () => {
      fireEvent.click(refreshBtn);
    });

    await waitFor(() => {
      expect(vi.mocked(skillRegistryApi.browse).mock.calls.length).toBeGreaterThan(callsBefore);
    });
    // The last browse call should use forceRefresh=true
    const calls = vi.mocked(skillRegistryApi.browse).mock.calls;
    const lastCall = calls[calls.length - 1];
    expect(lastCall[0]).toBe(true);
  });

  it('sorts hermes skills before non-hermes in installed view', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
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
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([alphaSkill, hermesSkill]);

    render(<SkillsExplorerTab />);
    await switchToInstalled();

    await waitFor(() => {
      expect(screen.getByText('Hermes Skill')).toBeInTheDocument();
    });

    const allTiles = screen
      .getAllByRole('button')
      .filter(el => el.getAttribute('data-testid')?.startsWith('skill-explorer-tile'));
    // Hermes should come first
    expect(allTiles[0]).toHaveAttribute('data-testid', 'skill-explorer-tile-hermes');
    expect(allTiles[1]).toHaveAttribute('data-testid', 'skill-explorer-tile-alpha');
  });

  it('activates catalog tile on Enter key', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    vi.mocked(skillRegistryApi.browse).mockResolvedValue([MOCK_CATALOG_ENTRY]);

    render(<SkillsExplorerTab />);

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
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const { skillRegistryApi } = await import('../../../services/api/skillRegistryApi');
    const onToast = vi.fn();
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    // Override the beforeEach mock so browse returns an entry
    vi.mocked(skillRegistryApi.browse).mockResolvedValue([MOCK_CATALOG_ENTRY]);
    vi.mocked(skillRegistryApi.install).mockResolvedValue({
      url: 'https://example.com/SKILL.md',
      stdout: 'ok',
      stderr: '',
      newSkills: [],
    });

    render(<SkillsExplorerTab onToast={onToast} />);

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
      expect(skillRegistryApi.install).toHaveBeenCalledWith('registry-skill-1');
    });
  });
});
