import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';

// [dev-workflow] Unit tests for DevWorkflowPanel.tsx — covers repo loading,
// not-connected error, fork detection, branch population, and save/clear wiring.

const hoisted = vi.hoisted(() => ({ composioExecute: vi.fn(), listConnections: vi.fn() }));

vi.mock('../../../../lib/composio/composioApi', () => ({
  execute: hoisted.composioExecute,
  listConnections: hoisted.listConnections,
}));

// Stable t function — creating a new function object on every render
// would cause useCallback([t]) to re-create on every render, triggering
// the loadRepos useEffect in an infinite loop.
const stableT = (key: string) => key;
vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: stableT }) }));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

vi.mock('../../components/SettingsHeader', () => ({
  default: ({ title }: { title: string }) => <div data-testid="settings-header">{title}</div>,
}));

// Import once — DevWorkflowPanel state is managed via API mocks and
// localStorage, not module-level vars, so a single import is sufficient.
async function importPanel() {
  const mod = await import('../DevWorkflowPanel');
  return mod.default;
}

// ── Mock data ─────────────────────────────────────────────────────────────────

const githubConnection = { connections: [{ id: 'conn-1', toolkit: 'github', status: 'ACTIVE' }] };

const reposResponse = {
  successful: true,
  data: [
    { full_name: 'user/repo1', name: 'repo1', owner: { login: 'user' }, private: false },
    { full_name: 'user/repo2', name: 'repo2', owner: { login: 'user' }, fork: true, private: true },
  ],
  error: null,
  costUsd: 0,
};

const repoMetaNonFork = {
  successful: true,
  data: { fork: false, default_branch: 'main' },
  error: null,
  costUsd: 0,
};

const repoMetaFork = {
  successful: true,
  data: {
    fork: true,
    parent: { full_name: 'upstream/repo', owner: { login: 'upstream' }, name: 'repo' },
    default_branch: 'main',
  },
  error: null,
  costUsd: 0,
};

const branchesResponse = {
  successful: true,
  data: { details: [{ name: 'main' }, { name: 'dev' }] },
  error: null,
  costUsd: 0,
};

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('DevWorkflowPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    hoisted.listConnections.mockResolvedValue(githubConnection);
    hoisted.composioExecute.mockResolvedValue(reposResponse);
  });

  test('renders header immediately and populates repo dropdown on successful fetch', async () => {
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // Header is rendered synchronously
    expect(screen.getByTestId('settings-header')).toBeInTheDocument();

    // Wait for repos to load
    await waitFor(() => {
      expect(screen.getByRole('option', { name: /user\/repo1/ })).toBeInTheDocument();
    });
    expect(screen.getByRole('option', { name: /user\/repo2/ })).toBeInTheDocument();

    expect(hoisted.composioExecute).toHaveBeenCalledWith(
      'GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER',
      {}
    );
  });

  test('shows not-connected error when no GitHub connection found', async () => {
    hoisted.listConnections.mockResolvedValue({ connections: [] });
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.errorNotConnected')).toBeInTheDocument();
    });
    // composioExecute should not be called if not connected
    expect(hoisted.composioExecute).not.toHaveBeenCalled();
  });

  test('shows not-connected error when connections list is missing', async () => {
    hoisted.listConnections.mockResolvedValue({});
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.errorNotConnected')).toBeInTheDocument();
    });
  });

  test('detects fork and shows upstream info after repo selection', async () => {
    // Call sequence: LIST_REPOS → GET_A_REPO (fork) → LIST_BRANCHES
    hoisted.composioExecute
      .mockResolvedValueOnce(reposResponse)
      .mockResolvedValueOnce(repoMetaFork)
      .mockResolvedValueOnce(branchesResponse);

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // Wait for repos to appear
    await waitFor(() => {
      expect(screen.getByRole('option', { name: /user\/repo1/ })).toBeInTheDocument();
    });

    // Select a repo
    const select = screen.getAllByRole('combobox')[0];
    fireEvent.change(select, { target: { value: 'user/repo1' } });

    // Fork info should appear
    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.forkDetected')).toBeInTheDocument();
    });
    expect(screen.getByText('upstream/repo')).toBeInTheDocument();
  });

  test('shows branches in dropdown after repo selection', async () => {
    // Call sequence: LIST_REPOS → GET_A_REPO (non-fork) → LIST_BRANCHES
    hoisted.composioExecute
      .mockResolvedValueOnce(reposResponse)
      .mockResolvedValueOnce(repoMetaNonFork)
      .mockResolvedValueOnce(branchesResponse);

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByRole('option', { name: /user\/repo1/ })).toBeInTheDocument();
    });

    const repoSelect = screen.getAllByRole('combobox')[0];
    fireEvent.change(repoSelect, { target: { value: 'user/repo1' } });

    await waitFor(() => {
      expect(screen.getByRole('option', { name: 'main' })).toBeInTheDocument();
    });
    expect(screen.getByRole('option', { name: 'dev' })).toBeInTheDocument();

    expect(hoisted.composioExecute).toHaveBeenCalledWith('GITHUB_LIST_BRANCHES', {
      owner: 'user',
      repo: 'repo1',
      per_page: 100,
    });
  });

  test('save button stores config in localStorage', async () => {
    // Call sequence: LIST_REPOS → GET_A_REPO (non-fork) → LIST_BRANCHES
    hoisted.composioExecute
      .mockResolvedValueOnce(reposResponse)
      .mockResolvedValueOnce(repoMetaNonFork)
      .mockResolvedValueOnce(branchesResponse);

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // Wait for repos
    await waitFor(() => {
      expect(screen.getByRole('option', { name: /user\/repo1/ })).toBeInTheDocument();
    });

    // Select repo
    const repoSelect = screen.getAllByRole('combobox')[0];
    fireEvent.change(repoSelect, { target: { value: 'user/repo1' } });

    // Wait for branches
    await waitFor(() => {
      expect(screen.getByRole('option', { name: 'main' })).toBeInTheDocument();
    });

    // Click save
    const saveBtn = screen.getByRole('button', {
      name: /settings\.devWorkflow\.(save|update)Configuration/,
    });
    fireEvent.click(saveBtn);

    // Verify localStorage was written
    const raw = localStorage.getItem('openhuman:dev-workflow-config');
    expect(raw).not.toBeNull();
    const stored = JSON.parse(raw!);
    expect(stored.repoFullName).toBe('user/repo1');
    expect(stored.repoOwner).toBe('user');
    expect(stored.repoName).toBe('repo1');
    expect(stored.targetBranch).toBe('main');
    expect(typeof stored.schedule).toBe('string');

    // Saved status indicator
    expect(screen.getByText('settings.devWorkflow.saved')).toBeInTheDocument();
  });

  test('remove button clears localStorage config', async () => {
    // Pre-populate localStorage so savedConfig is non-null on mount
    const existingConfig = {
      repoFullName: 'user/repo1',
      repoOwner: 'user',
      repoName: 'repo1',
      forkInfo: null,
      targetBranch: 'main',
      schedule: '*/30 * * * *',
    };
    localStorage.setItem('openhuman:dev-workflow-config', JSON.stringify(existingConfig));

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // Active config summary is shown immediately (initialised from localStorage)
    expect(screen.getByText('settings.devWorkflow.activeConfiguration')).toBeInTheDocument();

    // Remove button is visible because savedConfig is set
    const removeBtn = screen.getByRole('button', { name: 'settings.devWorkflow.remove' });
    fireEvent.click(removeBtn);

    // localStorage should be cleared
    expect(localStorage.getItem('openhuman:dev-workflow-config')).toBeNull();
    // Active config summary gone
    expect(screen.queryByText('settings.devWorkflow.activeConfiguration')).toBeNull();
  });

  test('shows branches fetched from upstream when fork is detected', async () => {
    // Call sequence: LIST_REPOS → GET_A_REPO (fork) → LIST_BRANCHES on upstream
    hoisted.composioExecute
      .mockResolvedValueOnce(reposResponse)
      .mockResolvedValueOnce(repoMetaFork)
      .mockResolvedValueOnce(branchesResponse);

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByRole('option', { name: /user\/repo1/ })).toBeInTheDocument();
    });

    const repoSelect = screen.getAllByRole('combobox')[0];
    fireEvent.change(repoSelect, { target: { value: 'user/repo1' } });

    await waitFor(() => {
      expect(screen.getByRole('option', { name: 'main' })).toBeInTheDocument();
    });

    // Branches were fetched from upstream owner/repo
    expect(hoisted.composioExecute).toHaveBeenCalledWith('GITHUB_LIST_BRANCHES', {
      owner: 'upstream',
      repo: 'repo',
      per_page: 100,
    });
  });

  test('panel still renders if listConnections rejects', async () => {
    hoisted.listConnections.mockRejectedValue(new Error('network error'));
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // Header always renders
    expect(screen.getByTestId('settings-header')).toBeInTheDocument();

    // Error state shown
    await waitFor(() => {
      expect(screen.getByText('network error')).toBeInTheDocument();
    });
  });
});
