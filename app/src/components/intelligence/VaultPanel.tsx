/**
 * Knowledge vaults — point the assistant at a local folder and have its
 * files mirrored into memory under namespace `vault:<id>`. Sits inside
 * the Intelligence ▸ Memory tab.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { ToastNotification } from '../../types/intelligence';
import { openUrl, revealPath } from '../../utils/openUrl';
import {
  type CoreVault,
  type CoreVaultSyncState,
  openhumanVaultCreate,
  openhumanVaultList,
  openhumanVaultRemove,
  openhumanVaultSync,
  openhumanVaultSyncStatus,
} from '../../utils/tauriCommands/vault';

/** How often the UI re-polls for sync progress while a sync is running (ms). */
const SYNC_POLL_INTERVAL_MS = 1_500;

interface VaultPanelProps {
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
}

export function VaultPanel({ onToast }: VaultPanelProps) {
  const { t } = useT();
  const [vaults, setVaults] = useState<CoreVault[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState<Record<string, 'sync' | 'remove' | undefined>>({});
  const [syncProgress, setSyncProgress] = useState<
    Record<string, { ingested: number; total: number } | undefined>
  >({});
  const [creating, setCreating] = useState(false);
  const [showForm, setShowForm] = useState(false);
  const [newName, setNewName] = useState('');
  const [newPath, setNewPath] = useState('');
  const [newExcludes, setNewExcludes] = useState('');

  // Track active polling timers so we can cancel them on unmount.
  const pollTimers = useRef<Record<string, ReturnType<typeof setTimeout>>>({});

  // Cancel all active poll timers on unmount.
  useEffect(() => {
    const timers = pollTimers.current;
    return () => {
      for (const t of Object.values(timers)) {
        clearTimeout(t);
      }
    };
  }, []);

  const reload = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const resp = await openhumanVaultList();
      setVaults(resp.result);
    } catch (err) {
      console.error('[ui-flow][vault-panel] list failed', err);
      setLoadError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const handleCreate = useCallback(
    async (event: React.FormEvent) => {
      event.preventDefault();
      const name = newName.trim();
      const rootPath = newPath.trim();
      if (!name || !rootPath) return;
      const excludeGlobs = newExcludes
        .split(',')
        .map(s => s.trim())
        .filter(Boolean);
      setCreating(true);
      try {
        const resp = await openhumanVaultCreate({ name, rootPath, excludeGlobs });
        onToast?.({
          type: 'success',
          title: t('vault.added'),
          message: t('vault.createdMessage')
            .replace('{name}', resp.result.name)
            .replace('{sync}', t('sync.sync')),
        });
        setNewName('');
        setNewPath('');
        setNewExcludes('');
        setShowForm(false);
        await reload();
      } catch (err) {
        console.error('[ui-flow][vault-panel] create failed', err);
        onToast?.({
          type: 'error',
          title: t('vault.couldNotAdd'),
          message: err instanceof Error ? err.message : String(err),
        });
      } finally {
        setCreating(false);
      }
    },
    [newExcludes, newName, newPath, onToast, reload, t]
  );

  const handleSync = useCallback(
    async (vault: CoreVault) => {
      setBusy(b => ({ ...b, [vault.id]: 'sync' }));
      setSyncProgress(p => ({ ...p, [vault.id]: undefined }));

      // Start the background sync.
      try {
        await openhumanVaultSync(vault.id);
      } catch (err) {
        console.error('[ui-flow][vault-panel] sync start failed', err);
        onToast?.({
          type: 'error',
          title: t('vault.syncFailed'),
          message: err instanceof Error ? err.message : String(err),
        });
        setBusy(b => ({ ...b, [vault.id]: undefined }));
        return;
      }

      console.debug('[ui-flow][vault-panel] sync started, polling for status', {
        vaultId: vault.id,
      });

      // Poll until the background task finishes.
      const vaultId = vault.id;
      const vaultName = vault.name;

      const poll = async () => {
        let st: CoreVaultSyncState;
        try {
          const resp = await openhumanVaultSyncStatus(vaultId);
          st = resp.result;
        } catch (err) {
          console.error('[ui-flow][vault-panel] sync status poll failed', err);
          setBusy(b => ({ ...b, [vaultId]: undefined }));
          setSyncProgress(p => ({ ...p, [vaultId]: undefined }));
          onToast?.({
            type: 'error',
            title: t('vault.syncFailed'),
            message: err instanceof Error ? err.message : String(err),
          });
          return;
        }

        // Update progress indicator while running.
        if (st.total > 0) {
          setSyncProgress(p => ({ ...p, [vaultId]: { ingested: st.ingested, total: st.total } }));
        }

        console.debug('[ui-flow][vault-panel] sync poll', {
          vaultId,
          status: st.status,
          ingested: st.ingested,
          total: st.total,
        });

        if (st.status === 'completed' || st.status === 'failed') {
          // Clear polling state and show final toast.
          delete pollTimers.current[vaultId];
          setBusy(b => ({ ...b, [vaultId]: undefined }));
          setSyncProgress(p => ({ ...p, [vaultId]: undefined }));

          if (st.status === 'failed') {
            onToast?.({
              type: 'error',
              title: t('vault.syncFailedFor').replace('{name}', vaultName),
              message:
                st.errors.length > 0
                  ? st.errors.slice(0, 3).join('; ')
                  : t('vault.syncFailedFiles').replace('{count}', String(st.failed)),
            });
          } else {
            onToast?.({
              type: st.failed > 0 ? 'info' : 'success',
              title: t('vault.syncedTitle').replace('{name}', vaultName),
              message: formatSyncSummary(st, t),
            });
          }
          await reload();
          return;
        }

        // Still running — schedule the next poll.
        pollTimers.current[vaultId] = setTimeout(() => {
          void poll();
        }, SYNC_POLL_INTERVAL_MS);
      };

      // First poll fires immediately (0 ms delay) so tests don't need fake timers.
      pollTimers.current[vaultId] = setTimeout(() => {
        void poll();
      }, 0);
    },
    [onToast, reload, t]
  );

  const handleRemove = useCallback(
    async (vault: CoreVault) => {
      const purge = window.confirm(
        t('vault.confirmRemovePurge')
          .replace('{name}', vault.name)
          .replace('{count}', String(vault.file_count))
      );
      // Confirm step #2: ensure the user actually meant to remove the vault row.
      const ok = window.confirm(t('vault.confirmRemove').replace('{name}', vault.name));
      if (!ok) return;
      setBusy(b => ({ ...b, [vault.id]: 'remove' }));
      try {
        await openhumanVaultRemove(vault.id, purge);
        onToast?.({
          type: 'success',
          title: t('vault.removed'),
          message: purge
            ? t('vault.removedPurgedMessage').replace('{name}', vault.name)
            : t('vault.removedKeptMessage').replace('{name}', vault.name),
        });
        await reload();
      } catch (err) {
        console.error('[ui-flow][vault-panel] remove failed', err);
        onToast?.({
          type: 'error',
          title: t('vault.couldNotRemove'),
          message: err instanceof Error ? err.message : String(err),
        });
      } finally {
        setBusy(b => ({ ...b, [vault.id]: undefined }));
      }
    },
    [onToast, reload, t]
  );

  const handleOpenVault = useCallback(
    async (rootPath: string) => {
      try {
        await openUrl(`obsidian://open?path=${encodeURIComponent(rootPath)}`);
        onToast?.({ type: 'info', title: t('vault.openSuccess'), message: rootPath });
        return;
      } catch (err) {
        console.error('[ui-flow][vault-panel] obsidian deep link failed', err);
      }
      try {
        await revealPath(rootPath);
        onToast?.({ type: 'info', title: t('vault.openFallback'), message: rootPath });
      } catch (err) {
        console.error('[ui-flow][vault-panel] reveal vault failed', err);
        onToast?.({ type: 'error', title: t('vault.openError'), message: String(err) });
      }
    },
    [onToast, t]
  );

  return (
    <div
      className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-4 shadow-sm"
      data-testid="vault-panel">
      <div className="mb-3 flex items-center justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold text-stone-800 dark:text-neutral-100">
            {t('vault.title')}
          </h3>
          <p className="text-xs text-stone-500 dark:text-neutral-400">{t('vault.description')}</p>
        </div>
        <button
          type="button"
          onClick={() => setShowForm(v => !v)}
          className="inline-flex items-center gap-1 rounded-md border border-primary-300 bg-white dark:bg-neutral-900
                     px-3 py-1.5 text-xs font-semibold text-primary-700 dark:text-primary-300 shadow-sm
                     transition-colors hover:bg-primary-50 dark:hover:bg-primary-500/15
                     focus:outline-none focus:ring-2 focus:ring-primary-200"
          data-testid="vault-add-toggle">
          {showForm ? t('common.cancel') : t('vault.add')}
        </button>
      </div>

      {showForm ? (
        <form
          onSubmit={handleCreate}
          className="mb-3 space-y-2 rounded-md border border-stone-100 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-3"
          data-testid="vault-add-form">
          <label className="block">
            <span className="text-xs font-medium text-stone-600 dark:text-neutral-300">
              {t('vault.name')}
            </span>
            <input
              type="text"
              value={newName}
              onChange={e => setNewName(e.target.value)}
              required
              placeholder={t('vault.namePlaceholder')}
              className="mt-1 w-full rounded border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-2 py-1.5 text-sm
                         focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
            />
          </label>
          <label className="block">
            <span className="text-xs font-medium text-stone-600 dark:text-neutral-300">
              {t('vault.folderPath')}
            </span>
            <input
              type="text"
              value={newPath}
              onChange={e => setNewPath(e.target.value)}
              required
              placeholder={t('vault.folderPathPlaceholder')}
              className="mt-1 w-full rounded border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-2 py-1.5 font-mono text-xs
                         focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
            />
          </label>
          <label className="block">
            <span className="text-xs font-medium text-stone-600 dark:text-neutral-300">
              {t('vault.excludes')}
            </span>
            <input
              type="text"
              value={newExcludes}
              onChange={e => setNewExcludes(e.target.value)}
              placeholder={t('vault.excludesPlaceholder')}
              className="mt-1 w-full rounded border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-2 py-1.5 text-xs
                         focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
            />
          </label>
          <div className="flex justify-end gap-2">
            <button
              type="submit"
              disabled={creating}
              className="rounded-md bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white
                         shadow-sm transition-colors hover:bg-primary-600
                         disabled:cursor-not-allowed disabled:opacity-50">
              {creating ? t('vault.creating') : t('vault.create')}
            </button>
          </div>
        </form>
      ) : null}

      {loading ? (
        <div className="py-4 text-center text-xs text-stone-400 dark:text-neutral-500">
          {t('vault.loading')}
        </div>
      ) : loadError ? (
        <div className="rounded border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-xs text-coral-800">
          {t('vault.failedToLoad').replace('{error}', loadError)}
        </div>
      ) : vaults.length === 0 ? (
        <div className="py-4 text-center text-xs text-stone-400 dark:text-neutral-500">
          {t('vault.empty')}
        </div>
      ) : (
        <ul className="divide-y divide-stone-100 dark:divide-neutral-800" data-testid="vault-list">
          {vaults.map(v => {
            const state = busy[v.id];
            const writeStateReason = v.write_state_reason
              ? t(
                  `vault.writeState.reasons.${v.write_state_reason}`,
                  t('vault.writeState.unknownReason')
                )
              : t('vault.writeState.unknownReason');
            return (
              <li key={v.id} className="flex items-center justify-between gap-3 py-2">
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium text-stone-800 dark:text-neutral-100">
                    {v.name}
                  </div>
                  <div
                    className="truncate font-mono text-[11px] text-stone-500 dark:text-neutral-400"
                    title={v.root_path}>
                    {v.root_path}
                  </div>
                  <div className="mt-0.5 text-[11px] text-stone-400 dark:text-neutral-500">
                    {t('vault.fileCount').replace('{count}', v.file_count.toLocaleString())} ·{' '}
                    {v.last_synced_at
                      ? t('vault.syncedRelative').replace(
                          '{time}',
                          formatRelative(v.last_synced_at, t)
                        )
                      : t('vault.neverSynced')}
                  </div>
                  <div className="mt-1 flex flex-wrap items-center gap-1.5">
                    <span
                      className={writeStateBadgeClass(v.write_state)}
                      title={writeStateReason}
                      data-testid={`vault-write-state-${v.id}`}>
                      {t(`vault.writeState.${v.write_state}`)}
                    </span>
                    <span className="text-[11px] text-stone-400 dark:text-neutral-500">
                      {writeStateReason}
                    </span>
                  </div>
                </div>
                <div className="flex items-center gap-2">
                  <button
                    type="button"
                    onClick={() => void handleOpenVault(v.root_path)}
                    disabled={state === 'sync' || state === 'remove'}
                    className="rounded-md border border-stone-300 bg-white dark:bg-neutral-900 px-3 py-1.5 text-xs
                               font-semibold text-stone-700 dark:text-neutral-200 shadow-sm transition-colors
                               hover:bg-stone-50 dark:hover:bg-neutral-800 disabled:cursor-not-allowed disabled:opacity-50"
                    data-testid="vault-open">
                    {t('vault.openButton')}
                  </button>
                  <button
                    type="button"
                    onClick={() => void handleSync(v)}
                    disabled={state === 'sync' || state === 'remove'}
                    className="rounded-md border border-primary-300 bg-white dark:bg-neutral-900 px-3 py-1.5 text-xs
                               font-semibold text-primary-700 dark:text-primary-300 shadow-sm transition-colors
                               hover:bg-primary-50 dark:hover:bg-primary-500/15 disabled:cursor-not-allowed disabled:opacity-50">
                    {state === 'sync'
                      ? (syncProgress[v.id]?.total ?? 0) > 0
                        ? t('vault.syncingProgress')
                            .replace('{ingested}', String(syncProgress[v.id]!.ingested))
                            .replace('{total}', String(syncProgress[v.id]!.total))
                        : t('sync.syncing')
                      : t('sync.sync')}
                  </button>
                  <button
                    type="button"
                    onClick={() => void handleRemove(v)}
                    disabled={state === 'sync' || state === 'remove'}
                    className="rounded-md border border-coral-200 dark:border-coral-500/30 bg-white dark:bg-neutral-900 px-3 py-1.5 text-xs
                               font-semibold text-coral-700 dark:text-coral-300 shadow-sm transition-colors
                               hover:bg-coral-50 dark:hover:bg-coral-500/10 disabled:cursor-not-allowed disabled:opacity-50">
                    {state === 'remove' ? t('vault.removing') : t('common.remove')}
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

function writeStateBadgeClass(state: CoreVault['write_state']): string {
  const base = 'inline-flex h-5 items-center rounded px-1.5 text-[10px] font-semibold';
  switch (state) {
    case 'writable':
      return `${base} bg-sage-50 text-sage-700 dark:bg-sage-500/10 dark:text-sage-300`;
    case 'read_only':
      return `${base} bg-amber-50 text-amber-700 dark:bg-amber-500/10 dark:text-amber-300`;
    case 'unavailable':
    default:
      return `${base} bg-coral-50 text-coral-700 dark:bg-coral-500/10 dark:text-coral-300`;
  }
}

function formatRelative(iso: string, translate: (key: string) => string): string {
  const timestamp = new Date(iso).getTime();
  if (Number.isNaN(timestamp)) return iso;
  const diff = Math.max(0, Date.now() - timestamp);
  const sec = Math.floor(diff / 1000);
  if (sec < 60) return translate('vault.relative.sec').replace('{count}', String(sec));
  const min = Math.floor(sec / 60);
  if (min < 60) return translate('vault.relative.min').replace('{count}', String(min));
  const hr = Math.floor(min / 60);
  if (hr < 24) return translate('vault.relative.hr').replace('{count}', String(hr));
  const day = Math.floor(hr / 24);
  return translate('vault.relative.day').replace('{count}', String(day));
}

function formatSyncSummary(state: CoreVaultSyncState, t: (key: string) => string): string {
  let summary = t('vault.syncSummary')
    .replace('{ingested}', String(state.ingested))
    .replace('{unchanged}', String(state.unchanged))
    .replace('{removed}', String(state.removed));
  if (state.failed > 0) {
    summary += t('vault.syncSummaryFailed').replace('{count}', String(state.failed));
  }
  if (state.skipped_unsupported > 0) {
    summary += t('vault.syncSummarySkipped').replace('{count}', String(state.skipped_unsupported));
  }
  if (state.duration_ms > 0) {
    summary += t('vault.syncSummaryDuration').replace(
      '{seconds}',
      (state.duration_ms / 1000).toFixed(1)
    );
  }
  return summary;
}
