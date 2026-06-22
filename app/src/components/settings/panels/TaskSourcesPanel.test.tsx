import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import TaskSourcesPanel from './TaskSourcesPanel';

const navigateBack = vi.fn();

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack, breadcrumbs: [] }),
}));

vi.mock('../components/SettingsHeader', () => ({
  default: ({ title }: { title: string }) => <div data-testid="settings-header">{title}</div>,
}));

const listMock = vi.fn();
const statusMock = vi.fn();
const addMock = vi.fn();
const updateMock = vi.fn();
const removeMock = vi.fn();
const fetchMock = vi.fn();
const syncMock = vi.fn();
const previewMock = vi.fn();
const listDatabasesMock = vi.fn();

vi.mock('../../../utils/tauriCommands', () => ({
  openhumanTaskSourcesList: () => listMock(),
  openhumanTaskSourcesStatus: () => statusMock(),
  openhumanTaskSourcesAdd: (p: unknown) => addMock(p),
  openhumanTaskSourcesUpdate: (id: string, patch: unknown) => updateMock(id, patch),
  openhumanTaskSourcesRemove: (id: string) => removeMock(id),
  openhumanTaskSourcesFetch: (id: string) => fetchMock(id),
  openhumanTaskSourcesSync: () => syncMock(),
  openhumanTaskSourcesPreviewFilter: (...args: unknown[]) => previewMock(...args),
  openhumanTaskSourcesListDatabases: (provider: string) => listDatabasesMock(provider),
}));

function sampleSource(overrides: Record<string, unknown> = {}) {
  return {
    id: 's-1',
    provider: 'github',
    name: 'My open issues',
    enabled: true,
    filter: { provider: 'github', repo: 'o/r', labels: [], assignee_is_me: true },
    intervalSecs: 1800,
    target: 'agent_todo_proactive',
    maxTasksPerFetch: 25,
    createdAt: '2025-01-01T00:00:00Z',
    ...overrides,
  };
}

function renderPanel() {
  return render(
    <MemoryRouter>
      <TaskSourcesPanel />
    </MemoryRouter>
  );
}

describe('<TaskSourcesPanel />', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    listMock.mockResolvedValue([sampleSource()]);
    statusMock.mockResolvedValue({
      enabled: true,
      defaultIntervalSecs: 1800,
      sourceCount: 1,
      enabledSourceCount: 1,
    });
    addMock.mockResolvedValue(sampleSource({ id: 's-2' }));
    updateMock.mockImplementation((id, patch) =>
      Promise.resolve(sampleSource({ id, ...(patch as object) }))
    );
    removeMock.mockResolvedValue({ id: 's-1', removed: true });
    fetchMock.mockResolvedValue({
      sourceId: 's-1',
      provider: 'github',
      fetched: 3,
      routed: 2,
      skippedDupe: 1,
      pruned: 0,
    });
    syncMock.mockResolvedValue([
      { sourceId: 's-1', provider: 'github', fetched: 3, routed: 2, skippedDupe: 1, pruned: 1 },
    ]);
    previewMock.mockResolvedValue([{ externalId: '1' }, { externalId: '2' }]);
    listDatabasesMock.mockResolvedValue([]);
  });

  afterEach(() => vi.restoreAllMocks());

  it('loads and renders configured sources', async () => {
    renderPanel();
    await waitFor(() => expect(listMock).toHaveBeenCalled());
    expect(await screen.findByTestId('task-source-s-1')).toBeInTheDocument();
    expect(screen.getByText('My open issues')).toBeInTheDocument();
    expect(statusMock).toHaveBeenCalled();
  });

  it('shows the empty state when there are no sources', async () => {
    listMock.mockResolvedValue([]);
    renderPanel();
    expect(await screen.findByText('No task sources configured yet.')).toBeInTheDocument();
  });

  it('shows the disabled banner when the domain is off', async () => {
    statusMock.mockResolvedValue({
      enabled: false,
      defaultIntervalSecs: 1800,
      sourceCount: 0,
      enabledSourceCount: 0,
    });
    listMock.mockResolvedValue([]);
    renderPanel();
    expect(
      await screen.findByText(
        'Task sources are disabled in settings. Enable them to poll automatically.'
      )
    ).toBeInTheDocument();
  });

  it('surfaces a load error', async () => {
    listMock.mockRejectedValue(new Error('boom'));
    renderPanel();
    await waitFor(() => expect(screen.getByText(/boom/)).toBeInTheDocument());
  });

  it('adds a source with the form-built filter', async () => {
    renderPanel();
    await screen.findByTestId('task-source-s-1');

    fireEvent.change(screen.getByPlaceholderText('e.g. My open issues'), {
      target: { value: 'My PRs' },
    });
    fireEvent.change(screen.getByLabelText('Repository (owner/name, optional)'), {
      target: { value: 'acme/app' },
    });
    fireEvent.change(screen.getByLabelText('Labels (comma-separated)'), {
      target: { value: 'bug, p1' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Add source' }));

    await waitFor(() => expect(addMock).toHaveBeenCalled());
    expect(addMock).toHaveBeenCalledWith({
      provider: 'github',
      name: 'My PRs',
      filter: { provider: 'github', repo: 'acme/app', labels: ['bug', 'p1'], assignee_is_me: true },
    });
    // List is reloaded after a successful add.
    expect(listMock).toHaveBeenCalledTimes(2);
  });

  it('previews the current filter without routing', async () => {
    renderPanel();
    await screen.findByTestId('task-source-s-1');
    fireEvent.click(screen.getByRole('button', { name: 'Preview' }));
    await waitFor(() => expect(previewMock).toHaveBeenCalled());
    expect(await screen.findByText(/2 task\(s\) match this filter/)).toBeInTheDocument();
  });

  it('toggles a source enabled state', async () => {
    renderPanel();
    const card = await screen.findByTestId('task-source-s-1');
    fireEvent.click(within(card).getByRole('button', { name: 'Disable' }));
    await waitFor(() => expect(updateMock).toHaveBeenCalledWith('s-1', { enabled: false }));
  });

  it('fetches now and surfaces the routed/fetched counts', async () => {
    renderPanel();
    const card = await screen.findByTestId('task-source-s-1');
    fireEvent.click(within(card).getByRole('button', { name: 'Fetch now' }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith('s-1'));
    expect(
      await screen.findByText(/Routed 2 of 3 task\(s\); removed 0 stale task\(s\)/)
    ).toBeInTheDocument();
  });

  it('syncs all sources and surfaces routed/fetched/pruned counts', async () => {
    renderPanel();
    await screen.findByTestId('task-source-s-1');
    fireEvent.click(screen.getByRole('button', { name: 'Sync all' }));
    await waitFor(() => expect(syncMock).toHaveBeenCalled());
    expect(
      await screen.findByText(/Routed 2 of 3 task\(s\); removed 1 stale task\(s\)/)
    ).toBeInTheDocument();
  });

  it('disables refresh while sync is in progress', async () => {
    let resolveSync: (value: Awaited<ReturnType<typeof syncMock>>) => void = () => {};
    syncMock.mockReturnValue(
      new Promise(resolve => {
        resolveSync = resolve;
      })
    );

    renderPanel();
    await screen.findByTestId('task-source-s-1');
    fireEvent.click(screen.getByRole('button', { name: 'Sync all' }));

    await waitFor(() => expect(syncMock).toHaveBeenCalled());
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeDisabled();

    resolveSync([
      { sourceId: 's-1', provider: 'github', fetched: 3, routed: 2, skippedDupe: 1, pruned: 1 },
    ]);
    await waitFor(() => expect(screen.getByRole('button', { name: 'Refresh' })).toBeEnabled());
  });

  it('surfaces a fetch outcome error', async () => {
    fetchMock.mockResolvedValue({
      sourceId: 's-1',
      provider: 'github',
      fetched: 0,
      routed: 0,
      skippedDupe: 0,
      pruned: 0,
      error: 'no connection',
    });
    renderPanel();
    const card = await screen.findByTestId('task-source-s-1');
    fireEvent.click(within(card).getByRole('button', { name: 'Fetch now' }));
    await waitFor(() => expect(screen.getByText('no connection')).toBeInTheDocument());
  });

  it('removes a source after confirm', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    renderPanel();
    const card = await screen.findByTestId('task-source-s-1');
    fireEvent.click(within(card).getByRole('button', { name: 'Remove' }));
    await waitFor(() => expect(removeMock).toHaveBeenCalledWith('s-1'));
    await waitFor(() => expect(screen.queryByTestId('task-source-s-1')).not.toBeInTheDocument());
  });

  it('does not remove a source when confirm is cancelled', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(false);
    renderPanel();
    const card = await screen.findByTestId('task-source-s-1');
    fireEvent.click(within(card).getByRole('button', { name: 'Remove' }));
    await waitFor(() => expect(window.confirm).toHaveBeenCalled());
    expect(removeMock).not.toHaveBeenCalled();
    expect(screen.getByTestId('task-source-s-1')).toBeInTheDocument();
  });

  it('switches the primary field label when the provider changes', async () => {
    renderPanel();
    await screen.findByTestId('task-source-s-1');
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'notion' } });
    expect(screen.getByLabelText('Database (board) ID')).toBeInTheDocument();
    // GitHub-only labels field is gone for Notion.
    expect(screen.queryByLabelText('Labels (comma-separated)')).not.toBeInTheDocument();
  });

  // ── New coverage for lines 417-418, 422, 425, 428-429, 464, 586 ──────────

  it('shows Notion Browse Databases button when notion provider is selected (lines 417-418)', async () => {
    renderPanel();
    await screen.findByTestId('task-source-s-1');
    // Switch to notion provider
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'notion' } });
    expect(screen.getByRole('button', { name: /Browse Databases/i })).toBeInTheDocument();
  });

  it('shows the database dropdown after Browse Databases returns results', async () => {
    listDatabasesMock.mockResolvedValue([{ id: 'db-1', title: 'My Notion DB' }]);
    renderPanel();
    await screen.findByTestId('task-source-s-1');

    // Switch the add-source provider to Notion so the Browse Databases control appears.
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'notion' } });
    fireEvent.click(screen.getByRole('button', { name: /Browse Databases/i }));

    // The returned database surfaces as a selectable option.
    expect(await screen.findByRole('option', { name: 'My Notion DB' })).toBeInTheDocument();
    expect(listDatabasesMock).toHaveBeenCalledWith('notion');
  });

  it('toggles "Assigned to me" checkbox and it affects state (line 464)', async () => {
    renderPanel();
    await screen.findByTestId('task-source-s-1');
    // The "Assigned to me" checkbox — it starts checked (assignedToMe defaults to true)
    const checkbox = screen.getByRole('checkbox', { name: /Assigned to me/i });
    expect(checkbox).toBeChecked();
    fireEvent.click(checkbox);
    expect(checkbox).not.toBeChecked();
  });

  it('clicking Refresh calls load() (line 586)', async () => {
    renderPanel();
    await screen.findByTestId('task-source-s-1');
    // The Refresh button early-returns inside load() while a load is in flight
    // (and is disabled in the UI for the same reason). Wait for the mount load
    // to finish — i.e. the button to become enabled — before clicking, mirroring
    // what a real user can do and removing the mount-vs-click race.
    const refreshButton = screen.getByRole('button', { name: /Refresh/i });
    await waitFor(() => expect(refreshButton).toBeEnabled());
    // The first listMock call happens on mount; clicking Refresh triggers another
    const beforeCount = listMock.mock.calls.length;
    fireEvent.click(refreshButton);
    await waitFor(() => {
      expect(listMock.mock.calls.length).toBeGreaterThan(beforeCount);
    });
  });
});
