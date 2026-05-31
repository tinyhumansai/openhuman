/**
 * Vitest for `<VaultPanel />`. Covers: load/empty/error states, the add-
 * vault form happy + error paths, per-row sync (success + failed-files
 * branch), and remove with both purge=true and purge=false flows.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { VaultPanel } from './VaultPanel';

const mockList = vi.fn();
const mockCreate = vi.fn();
const mockSync = vi.fn();
const mockSyncStatus = vi.fn();
const mockRemove = vi.fn();

const mockOpenUrl = vi.fn();
const mockRevealPath = vi.fn();

vi.mock('../../utils/tauriCommands/vault', () => ({
  openhumanVaultList: (...args: unknown[]) => mockList(...args),
  openhumanVaultCreate: (...args: unknown[]) => mockCreate(...args),
  openhumanVaultSync: (...args: unknown[]) => mockSync(...args),
  openhumanVaultSyncStatus: (...args: unknown[]) => mockSyncStatus(...args),
  openhumanVaultRemove: (...args: unknown[]) => mockRemove(...args),
}));

vi.mock('../../utils/openUrl', () => ({
  openUrl: (...args: unknown[]) => mockOpenUrl(...args),
  revealPath: (...args: unknown[]) => mockRevealPath(...args),
}));

function vault(overrides: Record<string, unknown> = {}) {
  return {
    id: 'v-1',
    name: 'Notes',
    root_path: '/Users/me/notes',
    namespace: 'vault:v-1',
    include_globs: [],
    exclude_globs: [],
    created_at: '2026-05-17T10:00:00Z',
    last_synced_at: null,
    file_count: 0,
    write_state: 'writable',
    write_state_reason: 'writable',
    ...overrides,
  };
}

/** Build a completed `CoreVaultSyncState` payload for mockSyncStatus. */
function syncState(overrides: Record<string, unknown> = {}) {
  return {
    result: {
      vault_id: 'v-1',
      status: 'completed',
      scanned: 4,
      ingested: 3,
      unchanged: 1,
      removed: 0,
      failed: 0,
      skipped_unsupported: 0,
      total: 4,
      started_at_ms: 1_000,
      finished_at_ms: 2_200,
      duration_ms: 1_200,
      errors: [],
      ...overrides,
    },
    logs: [],
  };
}

describe('<VaultPanel />', () => {
  beforeEach(() => {
    mockList.mockReset();
    mockCreate.mockReset();
    mockSync.mockReset();
    mockSyncStatus.mockReset();
    mockRemove.mockReset();
    mockOpenUrl.mockReset();
    mockRevealPath.mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('shows loading then empty state when list returns no vaults', async () => {
    mockList.mockResolvedValueOnce({ result: [], logs: [] });
    render(<VaultPanel />);
    expect(screen.getByText(/Loading vaults/)).toBeTruthy();
    await waitFor(() => screen.getByText(/No vaults yet/));
    expect(mockList).toHaveBeenCalledTimes(1);
  });

  it('renders an error banner when list fails', async () => {
    mockList.mockRejectedValueOnce(new Error('rpc down'));
    render(<VaultPanel />);
    await waitFor(() => screen.getByText(/Failed to load vaults/));
    expect(screen.getByText(/rpc down/)).toBeTruthy();
  });

  it('lists vaults with file count + relative last-synced label', async () => {
    vi.spyOn(Date, 'now').mockReturnValue(new Date('2026-05-17T10:05:00Z').getTime());
    mockList.mockResolvedValueOnce({
      result: [
        vault({ id: 'v-A', name: 'A', file_count: 42, last_synced_at: '2026-05-17T10:04:30Z' }),
      ],
      logs: [],
    });
    render(<VaultPanel />);
    await waitFor(() => screen.getByTestId('vault-list'));
    expect(screen.getByText('A')).toBeTruthy();
    expect(screen.getByText(/42 file/)).toBeTruthy();
    expect(screen.getByText(/synced 30s ago/)).toBeTruthy();
    expect(screen.getByTestId('vault-write-state-v-A')).toHaveTextContent('Writable');
    expect(screen.getByText(/Approved markdown/)).toBeTruthy();
  });

  it('shows read-only and unavailable vault write states with reasons', async () => {
    mockList.mockResolvedValueOnce({
      result: [
        vault({
          id: 'v-readonly',
          name: 'Read only',
          write_state: 'read_only',
          write_state_reason: 'read_only',
        }),
        vault({
          id: 'v-missing',
          name: 'Missing',
          write_state: 'unavailable',
          write_state_reason: 'unavailable',
        }),
      ],
      logs: [],
    });
    render(<VaultPanel />);
    await waitFor(() => screen.getByTestId('vault-list'));

    expect(screen.getByTestId('vault-write-state-v-readonly')).toHaveTextContent('Read-only');
    expect(screen.getByText('Vault folder is read-only on this device.')).toBeTruthy();
    expect(screen.getByTestId('vault-write-state-v-missing')).toHaveTextContent('Unavailable');
    expect(screen.getByText('Vault folder is not available on this device.')).toBeTruthy();
  });

  it('toggles the add form and creates a vault on submit', async () => {
    mockList
      .mockResolvedValueOnce({ result: [], logs: [] })
      .mockResolvedValueOnce({ result: [vault()], logs: [] });
    mockCreate.mockResolvedValueOnce({ result: vault(), logs: [] });
    const onToast = vi.fn();
    render(<VaultPanel onToast={onToast} />);
    await waitFor(() => screen.getByText(/No vaults yet/));

    fireEvent.click(screen.getByTestId('vault-add-toggle'));
    const form = screen.getByTestId('vault-add-form');
    const inputs = form.querySelectorAll('input');
    fireEvent.change(inputs[0], { target: { value: 'My notes' } });
    fireEvent.change(inputs[1], { target: { value: '/Users/me/notes' } });
    fireEvent.change(inputs[2], { target: { value: 'drafts, .secret' } });
    fireEvent.submit(form);

    await waitFor(() =>
      expect(mockCreate).toHaveBeenCalledWith({
        name: 'My notes',
        rootPath: '/Users/me/notes',
        excludeGlobs: ['drafts', '.secret'],
      })
    );
    expect(onToast).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'success', title: 'Vault added' })
    );
    // Reload happens after create — list called twice (initial + post-create).
    expect(mockList).toHaveBeenCalledTimes(2);
  });

  it('emits an error toast when create throws', async () => {
    mockList.mockResolvedValueOnce({ result: [], logs: [] });
    mockCreate.mockRejectedValueOnce(new Error('disk full'));
    const onToast = vi.fn();
    render(<VaultPanel onToast={onToast} />);
    await waitFor(() => screen.getByText(/No vaults yet/));

    fireEvent.click(screen.getByTestId('vault-add-toggle'));
    const form = screen.getByTestId('vault-add-form');
    const inputs = form.querySelectorAll('input');
    fireEvent.change(inputs[0], { target: { value: 'n' } });
    fireEvent.change(inputs[1], { target: { value: '/x' } });
    fireEvent.submit(form);

    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'error', title: 'Could not add vault' })
      )
    );
  });

  it('syncs a vault and reports counts via toast', async () => {
    mockList
      .mockResolvedValueOnce({ result: [vault()], logs: [] })
      .mockResolvedValueOnce({ result: [vault()], logs: [] });
    // vault_sync returns immediately with "started"
    mockSync.mockResolvedValueOnce({ result: { status: 'started', vault_id: 'v-1' }, logs: [] });
    // vault_sync_status returns "completed" on first poll
    mockSyncStatus.mockResolvedValueOnce(syncState());
    const onToast = vi.fn();
    render(<VaultPanel onToast={onToast} />);
    await waitFor(() => screen.getByTestId('vault-list'));

    fireEvent.click(screen.getByText('Sync'));
    await waitFor(() => expect(mockSync).toHaveBeenCalledWith('v-1'));
    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'success',
          title: expect.stringContaining('Synced'),
          message: expect.stringContaining('Ingested 3'),
        })
      )
    );
  });

  it('uses info toast when sync reports failed files', async () => {
    mockList
      .mockResolvedValueOnce({ result: [vault()], logs: [] })
      .mockResolvedValueOnce({ result: [vault()], logs: [] });
    mockSync.mockResolvedValueOnce({ result: { status: 'started', vault_id: 'v-1' }, logs: [] });
    mockSyncStatus.mockResolvedValueOnce(
      syncState({
        ingested: 1,
        unchanged: 0,
        failed: 1,
        duration_ms: 50,
        errors: ['x.md: read failed'],
      })
    );
    const onToast = vi.fn();
    render(<VaultPanel onToast={onToast} />);
    await waitFor(() => screen.getByTestId('vault-list'));

    fireEvent.click(screen.getByText('Sync'));
    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'info', message: expect.stringContaining('failed 1') })
      )
    );
  });

  it('emits error toast when sync RPC fails', async () => {
    mockList.mockResolvedValueOnce({ result: [vault()], logs: [] });
    mockSync.mockRejectedValueOnce(new Error('boom'));
    const onToast = vi.fn();
    render(<VaultPanel onToast={onToast} />);
    await waitFor(() => screen.getByTestId('vault-list'));

    fireEvent.click(screen.getByText('Sync'));
    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'error', title: 'Sync failed' })
      )
    );
  });

  it('emits error toast when sync status returns failed', async () => {
    mockList
      .mockResolvedValueOnce({ result: [vault()], logs: [] })
      .mockResolvedValueOnce({ result: [vault()], logs: [] });
    mockSync.mockResolvedValueOnce({ result: { status: 'started', vault_id: 'v-1' }, logs: [] });
    mockSyncStatus.mockResolvedValueOnce(
      syncState({ status: 'failed', failed: 0, errors: ['disk full'] })
    );
    const onToast = vi.fn();
    render(<VaultPanel onToast={onToast} />);
    await waitFor(() => screen.getByTestId('vault-list'));

    fireEvent.click(screen.getByText('Sync'));
    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'error',
          title: expect.stringContaining('Sync failed'),
          message: expect.stringContaining('disk full'),
        })
      )
    );
  });

  it('emits error toast when status poll RPC throws', async () => {
    mockList.mockResolvedValueOnce({ result: [vault()], logs: [] });
    mockSync.mockResolvedValueOnce({ result: { status: 'started', vault_id: 'v-1' }, logs: [] });
    mockSyncStatus.mockRejectedValueOnce(new Error('poll error'));
    const onToast = vi.fn();
    render(<VaultPanel onToast={onToast} />);
    await waitFor(() => screen.getByTestId('vault-list'));

    fireEvent.click(screen.getByText('Sync'));
    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'error', title: 'Sync failed', message: 'poll error' })
      )
    );
  });

  it('uses fallback failed-file count message when errors array is empty', async () => {
    mockList
      .mockResolvedValueOnce({ result: [vault()], logs: [] })
      .mockResolvedValueOnce({ result: [vault()], logs: [] });
    mockSync.mockResolvedValueOnce({ result: { status: 'started', vault_id: 'v-1' }, logs: [] });
    mockSyncStatus.mockResolvedValueOnce(syncState({ status: 'failed', failed: 3, errors: [] }));
    const onToast = vi.fn();
    render(<VaultPanel onToast={onToast} />);
    await waitFor(() => screen.getByTestId('vault-list'));

    fireEvent.click(screen.getByText('Sync'));
    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'error',
          message: expect.stringContaining('Failed 3 file(s)'),
        })
      )
    );
  });

  it('includes skipped_unsupported count in completed toast message', async () => {
    mockList
      .mockResolvedValueOnce({ result: [vault()], logs: [] })
      .mockResolvedValueOnce({ result: [vault()], logs: [] });
    mockSync.mockResolvedValueOnce({ result: { status: 'started', vault_id: 'v-1' }, logs: [] });
    mockSyncStatus.mockResolvedValueOnce(
      syncState({ ingested: 2, skipped_unsupported: 5, duration_ms: 0 })
    );
    const onToast = vi.fn();
    render(<VaultPanel onToast={onToast} />);
    await waitFor(() => screen.getByTestId('vault-list'));

    fireEvent.click(screen.getByText('Sync'));
    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({ message: expect.stringContaining('skipped 5') })
      )
    );
  });

  it('cancels pending poll timer on unmount', async () => {
    mockList.mockResolvedValueOnce({ result: [vault()], logs: [] });
    mockSync.mockResolvedValueOnce({ result: { status: 'started', vault_id: 'v-1' }, logs: [] });
    // Always running — the 1 500 ms re-poll timer stays live after poll #1.
    mockSyncStatus.mockResolvedValue(syncState({ status: 'running', ingested: 1, total: 4 }));

    const { unmount } = render(<VaultPanel />);
    await waitFor(() => screen.getByTestId('vault-list'));

    fireEvent.click(screen.getByText('Sync'));

    // Wait until the first poll fires (0 ms timer) so the 1 500 ms next-poll
    // timer is scheduled in pollTimers.current.
    await waitFor(() => expect(mockSyncStatus).toHaveBeenCalledTimes(1));

    const clearSpy = vi.spyOn(globalThis, 'clearTimeout');
    unmount();

    // useEffect cleanup must have called clearTimeout for the pending timer.
    expect(clearSpy).toHaveBeenCalled();
    clearSpy.mockRestore();
  });

  it('removes a vault with purge=true when both confirms accepted', async () => {
    mockList
      .mockResolvedValueOnce({ result: [vault()], logs: [] })
      .mockResolvedValueOnce({ result: [], logs: [] });
    mockRemove.mockResolvedValueOnce({
      result: { vault_id: 'v-1', removed: true, purged: true },
      logs: [],
    });
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true);
    const onToast = vi.fn();
    render(<VaultPanel onToast={onToast} />);
    await waitFor(() => screen.getByTestId('vault-list'));

    fireEvent.click(screen.getByText('Remove'));
    await waitFor(() => expect(mockRemove).toHaveBeenCalledWith('v-1', true));
    expect(onToast).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'success', message: expect.stringContaining('purged') })
    );
    confirmSpy.mockRestore();
  });

  it('removes a vault with purge=false when first confirm denied', async () => {
    mockList
      .mockResolvedValueOnce({ result: [vault()], logs: [] })
      .mockResolvedValueOnce({ result: [], logs: [] });
    mockRemove.mockResolvedValueOnce({
      result: { vault_id: 'v-1', removed: true, purged: false },
      logs: [],
    });
    // First confirm (purge?) → no; second confirm (really remove?) → yes.
    const confirmSpy = vi
      .spyOn(window, 'confirm')
      .mockReturnValueOnce(false)
      .mockReturnValueOnce(true);
    const onToast = vi.fn();
    render(<VaultPanel onToast={onToast} />);
    await waitFor(() => screen.getByTestId('vault-list'));

    fireEvent.click(screen.getByText('Remove'));
    await waitFor(() => expect(mockRemove).toHaveBeenCalledWith('v-1', false));
    expect(onToast).toHaveBeenCalledWith(
      expect.objectContaining({ message: expect.stringContaining('Documents kept') })
    );
    confirmSpy.mockRestore();
  });

  it('aborts remove when second confirm is denied', async () => {
    mockList.mockResolvedValueOnce({ result: [vault()], logs: [] });
    const confirmSpy = vi
      .spyOn(window, 'confirm')
      .mockReturnValueOnce(true)
      .mockReturnValueOnce(false);
    render(<VaultPanel />);
    await waitFor(() => screen.getByTestId('vault-list'));

    fireEvent.click(screen.getByText('Remove'));
    // Allow microtasks to settle so any (incorrect) RPC dispatch would land.
    await Promise.resolve();
    expect(mockRemove).not.toHaveBeenCalled();
    confirmSpy.mockRestore();
  });

  it('emits error toast when remove RPC fails', async () => {
    mockList.mockResolvedValueOnce({ result: [vault()], logs: [] });
    mockRemove.mockRejectedValueOnce(new Error('locked'));
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true);
    const onToast = vi.fn();
    render(<VaultPanel onToast={onToast} />);
    await waitFor(() => screen.getByTestId('vault-list'));

    fireEvent.click(screen.getByText('Remove'));
    await waitFor(() =>
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'error', title: 'Could not remove vault' })
      )
    );
    confirmSpy.mockRestore();
  });

  it('open vault fires obsidian deep link and shows success toast', async () => {
    mockList.mockResolvedValueOnce({ result: [vault()], logs: [] });
    mockOpenUrl.mockResolvedValueOnce(undefined);
    const onToast = vi.fn();
    render(<VaultPanel onToast={onToast} />);
    await waitFor(() => screen.getByTestId('vault-list'));

    fireEvent.click(screen.getByTestId('vault-open'));
    await waitFor(() =>
      expect(mockOpenUrl).toHaveBeenCalledWith(
        'obsidian://open?path=' + encodeURIComponent('/Users/me/notes')
      )
    );
    expect(onToast).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'info', title: 'Opened in Obsidian' })
    );
  });

  it('open vault falls back to revealPath when obsidian deep link fails', async () => {
    mockList.mockResolvedValueOnce({ result: [vault()], logs: [] });
    mockOpenUrl.mockRejectedValueOnce(new Error('scheme not handled'));
    mockRevealPath.mockResolvedValueOnce(undefined);
    const onToast = vi.fn();
    render(<VaultPanel onToast={onToast} />);
    await waitFor(() => screen.getByTestId('vault-list'));

    fireEvent.click(screen.getByTestId('vault-open'));
    await waitFor(() => expect(mockOpenUrl).toHaveBeenCalled());
    await waitFor(() => expect(mockRevealPath).toHaveBeenCalledWith('/Users/me/notes'));
    expect(onToast).toHaveBeenCalledWith(
      expect.objectContaining({
        type: 'info',
        title: 'Obsidian not found — opened in file manager',
      })
    );
  });

  it('open vault shows error toast when both obsidian and reveal fail', async () => {
    mockList.mockResolvedValueOnce({ result: [vault()], logs: [] });
    mockOpenUrl.mockRejectedValueOnce(new Error('scheme not handled'));
    mockRevealPath.mockRejectedValueOnce(new Error('permission denied'));
    const onToast = vi.fn();
    render(<VaultPanel onToast={onToast} />);
    await waitFor(() => screen.getByTestId('vault-list'));

    fireEvent.click(screen.getByTestId('vault-open'));
    await waitFor(() => expect(mockOpenUrl).toHaveBeenCalled());
    await waitFor(() => expect(mockRevealPath).toHaveBeenCalled());
    expect(onToast).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'error', title: "Couldn't open vault" })
    );
  });
});
