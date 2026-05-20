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
const mockRemove = vi.fn();

vi.mock('../../utils/tauriCommands/vault', () => ({
  openhumanVaultList: (...args: unknown[]) => mockList(...args),
  openhumanVaultCreate: (...args: unknown[]) => mockCreate(...args),
  openhumanVaultSync: (...args: unknown[]) => mockSync(...args),
  openhumanVaultRemove: (...args: unknown[]) => mockRemove(...args),
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
    ...overrides,
  };
}

describe('<VaultPanel />', () => {
  beforeEach(() => {
    mockList.mockReset();
    mockCreate.mockReset();
    mockSync.mockReset();
    mockRemove.mockReset();
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
    mockSync.mockResolvedValueOnce({
      result: {
        vault_id: 'v-1',
        scanned: 4,
        ingested: 3,
        unchanged: 1,
        removed: 0,
        failed: 0,
        skipped_unsupported: 0,
        duration_ms: 1200,
        errors: [],
      },
      logs: [],
    });
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
    mockSync.mockResolvedValueOnce({
      result: {
        vault_id: 'v-1',
        scanned: 2,
        ingested: 1,
        unchanged: 0,
        removed: 0,
        failed: 1,
        skipped_unsupported: 0,
        duration_ms: 50,
        errors: ['x.md: read failed'],
      },
      logs: [],
    });
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
});
