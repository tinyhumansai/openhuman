/**
 * Unit tests for MemorySourcesRegistry — All In button, gear/settings panel,
 * per-kind field visibility, Save, and existing toggle behaviour.
 */
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import * as service from '../../../services/memorySourcesService';
import { getCoreStateSnapshot } from '../../../lib/coreState/store';
import { CoreStateContext, type CoreStateContextValue } from '../../../providers/coreStateContext';
import type { MemorySourceEntry } from '../../../services/memorySourcesService';
import { createTestStore, renderWithProviders } from '../../../test/test-utils';
import {
  openhumanGetMemorySyncSettings,
  openhumanUpdateMemorySyncSettings,
} from '../../../utils/tauriCommands/config';
import { MemorySourcesRegistry } from '../MemorySourcesRegistry';

// Mock the entire service so we don't hit RPC
vi.mock('../../../services/memorySourcesService', async () => {
  const actual = await vi.importActual<typeof import('../../../services/memorySourcesService')>(
    '../../../services/memorySourcesService'
  );
  return {
    ...actual,
    listMemorySources: vi.fn(),
    memorySourcesStatusList: vi.fn(),
    updateMemorySource: vi.fn(),
    removeMemorySource: vi.fn(),
    syncMemorySource: vi.fn(),
    applyAllIn: vi.fn(),
  };
});

// Mock tauriCommands/memoryTree — not needed in these tests
vi.mock('../../../utils/tauriCommands/memoryTree', () => ({
  memoryTreeFlushSource: vi.fn().mockResolvedValue({ seals_fired: 0 }),
}));

// Mock the memory-sync schedule config RPCs (#3302).
vi.mock('../../../utils/tauriCommands/config', () => ({
  openhumanGetMemorySyncSettings: vi.fn(),
  openhumanUpdateMemorySyncSettings: vi.fn(),
}));

const mockedList = vi.mocked(service.listMemorySources);
const mockedStatus = vi.mocked(service.memorySourcesStatusList);
const mockedUpdate = vi.mocked(service.updateMemorySource);
const mockedApplyAllIn = vi.mocked(service.applyAllIn);
const mockedGetSync = vi.mocked(openhumanGetMemorySyncSettings);
const mockedUpdateSync = vi.mocked(openhumanUpdateMemorySyncSettings);

function syncSettings(overrides: Record<string, unknown> = {}) {
  return {
    result: {
      sync_interval_secs: null,
      selected_secs: 86_400,
      is_manual: false,
      is_default: true,
      default_secs: 86_400,
      presets: [14_400, 43_200, 86_400],
      ...overrides,
    },
    logs: [],
  };
}

function makeSource(overrides: Partial<MemorySourceEntry> = {}): MemorySourceEntry {
  return {
    id: 'src_1',
    kind: 'github_repo',
    label: 'My Repo',
    enabled: true,
    url: 'https://github.com/org/repo',
    ...overrides,
  };
}

function setup(sources: MemorySourceEntry[] = [makeSource()]) {
  mockedList.mockResolvedValue(sources);
  mockedStatus.mockResolvedValue([]);
  const onToast = vi.fn();
  const result = renderWithProviders(
    <MemorySourcesRegistry onToast={onToast} pollIntervalMs={0} />,
    {}
  );
  return { ...result, onToast };
}

describe('MemorySourcesRegistry', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Default: schedule RPCs succeed with the unset/default 24h view.
    mockedGetSync.mockResolvedValue(syncSettings());
    mockedUpdateSync.mockResolvedValue(syncSettings());
  });

  // -------------------------------------------------------------------------
  // Basic render
  // -------------------------------------------------------------------------
  it('renders loaded sources list', async () => {
    setup([makeSource({ label: 'Work Repo' })]);
    await screen.findByText('Work Repo');
    expect(screen.getByTestId('memory-sources')).toBeInTheDocument();
  });

  it('renders empty state when no sources', async () => {
    mockedList.mockResolvedValue([]);
    mockedStatus.mockResolvedValue([]);
    renderWithProviders(<MemorySourcesRegistry pollIntervalMs={0} />);
    await screen.findByText(/no memory sources/i);
  });

  // -------------------------------------------------------------------------
  // Toggle (existing behaviour)
  // -------------------------------------------------------------------------
  it('toggle calls updateMemorySource and flips state', async () => {
    const source = makeSource({ enabled: true });
    mockedUpdate.mockResolvedValue({ ...source, enabled: false });
    setup([source]);
    await screen.findByText('My Repo');

    const toggle = screen.getByTitle(/disable/i);
    fireEvent.click(toggle);

    await waitFor(() => {
      expect(mockedUpdate).toHaveBeenCalledWith('src_1', { enabled: false });
    });
  });

  // -------------------------------------------------------------------------
  // All In button
  // -------------------------------------------------------------------------
  it('All In button is rendered in the header', async () => {
    setup();
    await screen.findByText('My Repo');
    expect(screen.getByTestId('all-in-button')).toBeInTheDocument();
  });

  it('clicking All In opens a confirmation modal', async () => {
    setup();
    await screen.findByText('My Repo');

    fireEvent.click(screen.getByTestId('all-in-button'));

    // The modal should appear
    await screen.findByText('Go All In?');
    expect(
      screen.getByText(/This enables every memory source and removes all sync limits/i)
    ).toBeInTheDocument();
  });

  it('cancelling All In modal closes it without calling applyAllIn', async () => {
    setup();
    await screen.findByText('My Repo');

    fireEvent.click(screen.getByTestId('all-in-button'));
    await screen.findByText('Go All In?');

    // Click the No / cancel button
    fireEvent.click(screen.getByText('No'));

    await waitFor(() => {
      expect(screen.queryByText('Go All In?')).not.toBeInTheDocument();
    });
    expect(mockedApplyAllIn).not.toHaveBeenCalled();
  });

  it('confirming All In calls applyAllIn, updates sources, and shows success toast', async () => {
    const updatedSrc = makeSource({ id: 'src_2', label: 'New Repo', enabled: true });
    mockedApplyAllIn.mockResolvedValue({ sources: [updatedSrc], sync_triggered: 1 });

    const { onToast } = setup();
    await screen.findByText('My Repo');

    fireEvent.click(screen.getByTestId('all-in-button'));
    await screen.findByText('Go All In?');

    fireEvent.click(screen.getByText('Yes'));

    await waitFor(() => {
      expect(mockedApplyAllIn).toHaveBeenCalledOnce();
    });

    await waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'success' }));
    });

    // Modal should close
    await waitFor(() => {
      expect(screen.queryByText('Go All In?')).not.toBeInTheDocument();
    });
  });

  it('All In failure shows error toast', async () => {
    mockedApplyAllIn.mockRejectedValue(new Error('RPC error'));

    const { onToast } = setup();
    await screen.findByText('My Repo');

    fireEvent.click(screen.getByTestId('all-in-button'));
    await screen.findByText('Go All In?');

    fireEvent.click(screen.getByText('Yes'));

    await waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }));
    });
  });

  // -------------------------------------------------------------------------
  // Gear / settings panel — toggling
  // -------------------------------------------------------------------------
  it('gear button renders for each source row', async () => {
    setup([makeSource({ id: 'src_1' }), makeSource({ id: 'src_2', label: 'Second' })]);
    await screen.findByText('My Repo');

    expect(screen.getByTestId('memory-source-settings-src_1')).toBeInTheDocument();
    expect(screen.getByTestId('memory-source-settings-src_2')).toBeInTheDocument();
  });

  it('clicking gear expands the settings panel for that source', async () => {
    setup([makeSource({ id: 'src_1', kind: 'github_repo' })]);
    await screen.findByText('My Repo');

    expect(screen.queryByTestId('source-settings-panel-src_1')).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId('memory-source-settings-src_1'));

    expect(screen.getByTestId('source-settings-panel-src_1')).toBeInTheDocument();
  });

  it('clicking gear again collapses the settings panel', async () => {
    setup([makeSource({ id: 'src_1', kind: 'github_repo' })]);
    await screen.findByText('My Repo');

    const gearBtn = screen.getByTestId('memory-source-settings-src_1');
    fireEvent.click(gearBtn);
    expect(screen.getByTestId('source-settings-panel-src_1')).toBeInTheDocument();

    fireEvent.click(gearBtn);
    expect(screen.queryByTestId('source-settings-panel-src_1')).not.toBeInTheDocument();
  });

  // -------------------------------------------------------------------------
  // Settings panel — field visibility per kind
  // -------------------------------------------------------------------------
  it('github_repo settings panel shows max_prs, max_issues, max_commits, sync_depth_days', async () => {
    setup([makeSource({ id: 'src_1', kind: 'github_repo' })]);
    await screen.findByText('My Repo');

    fireEvent.click(screen.getByTestId('memory-source-settings-src_1'));

    const panel = screen.getByTestId('source-settings-panel-src_1');
    expect(within(panel).getByLabelText(/max pull requests/i)).toBeInTheDocument();
    expect(within(panel).getByLabelText(/max issues/i)).toBeInTheDocument();
    expect(within(panel).getByLabelText(/max commits/i)).toBeInTheDocument();
    expect(within(panel).getByLabelText(/sync depth/i)).toBeInTheDocument();
  });

  it('composio settings panel shows sync_depth_days and max_items but NOT max_prs', async () => {
    setup([makeSource({ id: 'src_1', kind: 'composio', toolkit: 'github' })]);
    await screen.findByText('My Repo');

    fireEvent.click(screen.getByTestId('memory-source-settings-src_1'));

    const panel = screen.getByTestId('source-settings-panel-src_1');
    expect(within(panel).getByLabelText(/max items/i)).toBeInTheDocument();
    expect(within(panel).getByLabelText(/sync depth/i)).toBeInTheDocument();
    expect(within(panel).queryByLabelText(/max pull requests/i)).not.toBeInTheDocument();
  });

  it('rss_feed settings panel shows max_items and sync_depth_days but NOT max_prs', async () => {
    setup([makeSource({ id: 'src_1', kind: 'rss_feed', url: 'https://example.com/feed.xml' })]);
    await screen.findByText('My Repo');

    fireEvent.click(screen.getByTestId('memory-source-settings-src_1'));

    const panel = screen.getByTestId('source-settings-panel-src_1');
    expect(within(panel).getByLabelText(/max items/i)).toBeInTheDocument();
    expect(within(panel).getByLabelText(/sync depth/i)).toBeInTheDocument();
    expect(within(panel).queryByLabelText(/max pull requests/i)).not.toBeInTheDocument();
    expect(within(panel).queryByLabelText(/max commits/i)).not.toBeInTheDocument();
  });

  // -------------------------------------------------------------------------
  // Settings panel — Save
  // -------------------------------------------------------------------------
  it('Save in settings panel calls updateMemorySource with numeric patch', async () => {
    const source = makeSource({ id: 'src_1', kind: 'github_repo' });
    const updated = { ...source, max_prs: 50 };
    mockedUpdate.mockResolvedValue(updated);

    const { onToast } = setup([source]);
    await screen.findByText('My Repo');

    fireEvent.click(screen.getByTestId('memory-source-settings-src_1'));

    const panel = screen.getByTestId('source-settings-panel-src_1');
    const maxPrsInput = within(panel).getByLabelText(/max pull requests/i);
    fireEvent.change(maxPrsInput, { target: { value: '50' } });

    const saveBtn = within(panel).getByText('Save');
    fireEvent.click(saveBtn);

    await waitFor(() => {
      expect(mockedUpdate).toHaveBeenCalledWith('src_1', expect.objectContaining({ max_prs: 50 }));
    });

    await waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'success' }));
    });
  });

  it('empty input is omitted from the save patch (not sent as 0)', async () => {
    const source = makeSource({ id: 'src_1', kind: 'github_repo', max_prs: 10 });
    mockedUpdate.mockResolvedValue(source);

    setup([source]);
    await screen.findByText('My Repo');

    fireEvent.click(screen.getByTestId('memory-source-settings-src_1'));

    const panel = screen.getByTestId('source-settings-panel-src_1');
    const maxPrsInput = within(panel).getByLabelText(/max pull requests/i);

    // Clear the field
    fireEvent.change(maxPrsInput, { target: { value: '' } });

    fireEvent.click(within(panel).getByText('Save'));

    await waitFor(() => {
      expect(mockedUpdate).toHaveBeenCalledWith(
        'src_1',
        expect.not.objectContaining({ max_prs: expect.anything() })
      );
    });
  });

  it('Save failure shows error toast', async () => {
    mockedUpdate.mockRejectedValue(new Error('Save failed'));

    const { onToast } = setup([makeSource({ kind: 'github_repo' })]);
    await screen.findByText('My Repo');

    fireEvent.click(screen.getByTestId('memory-source-settings-src_1'));
    const panel = screen.getByTestId('source-settings-panel-src_1');
    fireEvent.click(within(panel).getByText('Save'));

    await waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }));
    });
  });

  // -------------------------------------------------------------------------
  // Memory sync schedule (#3302)
  // -------------------------------------------------------------------------
  it('renders the sync schedule with the default 24h cadence highlighted', async () => {
    setup();
    const schedule = await screen.findByTestId('memory-sync-schedule');
    // Default view highlights the 24h preset.
    const preset24h = within(schedule).getByTestId('memory-sync-preset-86400');
    expect(preset24h).toHaveAttribute('aria-checked', 'true');
    // Manual is offered and not selected.
    const manual = within(schedule).getByTestId('memory-sync-preset-0');
    expect(manual).toHaveAttribute('aria-checked', 'false');
    // The summary shows the current cadence.
    expect(within(schedule).getByTestId('memory-sync-current')).toHaveTextContent('Every 24h');
  });

  it('shows "Last synced …" from the newest source chunk timestamp', async () => {
    mockedList.mockResolvedValue([makeSource()]);
    mockedStatus.mockResolvedValue([
      {
        source_id: 'src_1',
        chunks_synced: 5,
        chunks_pending: 0,
        last_chunk_at_ms: Date.now() - 2 * 60 * 60 * 1000,
        freshness: 'recent',
      },
    ]);
    renderWithProviders(<MemorySourcesRegistry pollIntervalMs={0} />);
    const schedule = await screen.findByTestId('memory-sync-schedule');
    await waitFor(() => {
      expect(schedule).toHaveTextContent(/Last synced/i);
      expect(schedule).toHaveTextContent(/2h ago/i);
    });
  });

  it('selecting a preset calls update with the cadence in seconds', async () => {
    setup();
    const schedule = await screen.findByTestId('memory-sync-schedule');

    mockedUpdateSync.mockResolvedValue(
      syncSettings({ sync_interval_secs: 14_400, selected_secs: 14_400, is_default: false })
    );
    fireEvent.click(within(schedule).getByTestId('memory-sync-preset-14400'));

    await waitFor(() => {
      expect(mockedUpdateSync).toHaveBeenCalledWith({ sync_interval_secs: 14_400 });
    });
    await waitFor(() => {
      expect(within(schedule).getByTestId('memory-sync-current')).toHaveTextContent('Every 4h');
    });
  });

  it('selecting Manual sends sync_interval_secs = 0 and shows Manual only', async () => {
    setup();
    const schedule = await screen.findByTestId('memory-sync-schedule');

    mockedUpdateSync.mockResolvedValue(
      syncSettings({ sync_interval_secs: 0, selected_secs: 0, is_manual: true, is_default: false })
    );
    fireEvent.click(within(schedule).getByTestId('memory-sync-preset-0'));

    await waitFor(() => {
      expect(mockedUpdateSync).toHaveBeenCalledWith({ sync_interval_secs: 0 });
    });
    await waitFor(() => {
      expect(within(schedule).getByTestId('memory-sync-current')).toHaveTextContent('Manual only');
    });
  });

  it('schedule update failure shows an error toast', async () => {
    const { onToast } = setup();
    const schedule = await screen.findByTestId('memory-sync-schedule');

    mockedUpdateSync.mockRejectedValue(new Error('RPC down'));
    fireEvent.click(within(schedule).getByTestId('memory-sync-preset-14400'));

    await waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }));
    });
  });

  // -------------------------------------------------------------------------
  // Refetch when the session becomes authenticated (#3449)
  //
  // After a page reload the registry can mount before CoreStateProvider has
  // restored the session. The first fetch then runs against a not-yet-ready
  // core; sources must reappear as soon as auth flips true, without waiting
  // for the next background poll.
  // -------------------------------------------------------------------------
  it('refetches sources when the session transitions to authenticated', async () => {
    mockedList.mockResolvedValue([makeSource({ label: 'Reloaded Repo' })]);
    mockedStatus.mockResolvedValue([]);

    const coreState = (isAuthenticated: boolean): CoreStateContextValue =>
      ({
        ...getCoreStateSnapshot(),
        snapshot: {
          ...getCoreStateSnapshot().snapshot,
          auth: {
            isAuthenticated,
            userId: isAuthenticated ? 'u1' : null,
            user: null,
            profileId: null,
          },
        },
        refresh: async () => {},
        refreshTeams: async () => {},
        refreshTeamMembers: async () => {},
        refreshTeamInvites: async () => {},
        setAnalyticsEnabled: async () => {},
        setMeetAutoOrchestratorHandoff: async () => {},
        setOnboardingCompletedFlag: async () => {},
        setEncryptionKey: async () => {},
        patchSnapshot: () => {},
        setOnboardingTasks: async () => {},
        storeSessionToken: async () => {},
        clearSession: async () => {},
      }) as CoreStateContextValue;

    const store = createTestStore();
    const tree = (isAuthenticated: boolean) => (
      <Provider store={store}>
        <CoreStateContext.Provider value={coreState(isAuthenticated)}>
          <MemoryRouter>
            {/* pollIntervalMs=0 disables the background poll, so the only way
                the second fetch can fire is the auth transition. */}
            <MemorySourcesRegistry pollIntervalMs={0} />
          </MemoryRouter>
        </CoreStateContext.Provider>
      </Provider>
    );

    const { rerender } = render(tree(false));
    await waitFor(() => expect(mockedList).toHaveBeenCalledTimes(1));

    rerender(tree(true));
    await waitFor(() => expect(mockedList).toHaveBeenCalledTimes(2));
    expect(await screen.findByText('Reloaded Repo')).toBeInTheDocument();
  });
});
