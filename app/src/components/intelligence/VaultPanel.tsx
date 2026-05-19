/**
 * Knowledge vaults — point the assistant at a local folder and have its
 * files mirrored into memory under namespace `vault:<id>`. Sits inside
 * the Intelligence ▸ Memory tab.
 */
import { useCallback, useEffect, useState } from 'react';

import type { ToastNotification } from '../../types/intelligence';
import {
  type CoreVault,
  type CoreVaultSyncReport,
  openhumanVaultCreate,
  openhumanVaultList,
  openhumanVaultRemove,
  openhumanVaultSync,
} from '../../utils/tauriCommands/vault';

interface VaultPanelProps {
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
}

export function VaultPanel({ onToast }: VaultPanelProps) {
  const [vaults, setVaults] = useState<CoreVault[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState<Record<string, 'sync' | 'remove' | undefined>>({});
  const [creating, setCreating] = useState(false);
  const [showForm, setShowForm] = useState(false);
  const [newName, setNewName] = useState('');
  const [newPath, setNewPath] = useState('');
  const [newExcludes, setNewExcludes] = useState('');

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
          title: 'Vault added',
          message: `Created "${resp.result.name}". Click Sync to ingest.`,
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
          title: 'Could not add vault',
          message: err instanceof Error ? err.message : String(err),
        });
      } finally {
        setCreating(false);
      }
    },
    [newName, newPath, newExcludes, onToast, reload]
  );

  const handleSync = useCallback(
    async (vault: CoreVault) => {
      setBusy(b => ({ ...b, [vault.id]: 'sync' }));
      try {
        const resp = await openhumanVaultSync(vault.id);
        const r: CoreVaultSyncReport = resp.result;
        onToast?.({
          type: r.failed > 0 ? 'info' : 'success',
          title: `Synced "${vault.name}"`,
          message:
            `Ingested ${r.ingested}, unchanged ${r.unchanged}, removed ${r.removed}` +
            (r.failed > 0 ? `, failed ${r.failed}` : '') +
            (r.skipped_unsupported > 0 ? `, skipped ${r.skipped_unsupported}` : '') +
            ` · ${(r.duration_ms / 1000).toFixed(1)}s`,
        });
        await reload();
      } catch (err) {
        console.error('[ui-flow][vault-panel] sync failed', err);
        onToast?.({
          type: 'error',
          title: 'Sync failed',
          message: err instanceof Error ? err.message : String(err),
        });
      } finally {
        setBusy(b => ({ ...b, [vault.id]: undefined }));
      }
    },
    [onToast, reload]
  );

  const handleRemove = useCallback(
    async (vault: CoreVault) => {
      const purge = window.confirm(
        `Remove vault "${vault.name}"?\n\nClick OK to also purge its memory (delete all ${vault.file_count} ingested document(s)).\nClick Cancel to keep the documents in memory.`
      );
      // Confirm step #2: ensure the user actually meant to remove the vault row.
      const ok = window.confirm(`Really remove vault "${vault.name}"?`);
      if (!ok) return;
      setBusy(b => ({ ...b, [vault.id]: 'remove' }));
      try {
        await openhumanVaultRemove(vault.id, purge);
        onToast?.({
          type: 'success',
          title: 'Vault removed',
          message: purge
            ? `Removed "${vault.name}" and purged its memory.`
            : `Removed "${vault.name}". Documents kept in memory.`,
        });
        await reload();
      } catch (err) {
        console.error('[ui-flow][vault-panel] remove failed', err);
        onToast?.({
          type: 'error',
          title: 'Could not remove vault',
          message: err instanceof Error ? err.message : String(err),
        });
      } finally {
        setBusy(b => ({ ...b, [vault.id]: undefined }));
      }
    },
    [onToast, reload]
  );

  return (
    <div
      className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-4 shadow-sm"
      data-testid="vault-panel">
      <div className="mb-3 flex items-center justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold text-stone-800 dark:text-neutral-100">
            Knowledge vaults
          </h3>
          <p className="text-xs text-stone-500 dark:text-neutral-400">
            Point at a local folder; files are chunked and mirrored into memory.
          </p>
        </div>
        <button
          type="button"
          onClick={() => setShowForm(v => !v)}
          className="inline-flex items-center gap-1 rounded-md border border-primary-300 bg-white dark:bg-neutral-900
                     px-3 py-1.5 text-xs font-semibold text-primary-700 dark:text-primary-300 shadow-sm
                     transition-colors hover:bg-primary-50 dark:hover:bg-primary-500/15
                     focus:outline-none focus:ring-2 focus:ring-primary-200"
          data-testid="vault-add-toggle">
          {showForm ? 'Cancel' : '+ Add vault'}
        </button>
      </div>

      {showForm ? (
        <form
          onSubmit={handleCreate}
          className="mb-3 space-y-2 rounded-md border border-stone-100 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-3"
          data-testid="vault-add-form">
          <label className="block">
            <span className="text-xs font-medium text-stone-600 dark:text-neutral-300">Name</span>
            <input
              type="text"
              value={newName}
              onChange={e => setNewName(e.target.value)}
              required
              placeholder="My research notes"
              className="mt-1 w-full rounded border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-2 py-1.5 text-sm
                         focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
            />
          </label>
          <label className="block">
            <span className="text-xs font-medium text-stone-600 dark:text-neutral-300">
              Folder path (absolute)
            </span>
            <input
              type="text"
              value={newPath}
              onChange={e => setNewPath(e.target.value)}
              required
              placeholder="/Users/you/Documents/notes"
              className="mt-1 w-full rounded border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-2 py-1.5 font-mono text-xs
                         focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-200"
            />
          </label>
          <label className="block">
            <span className="text-xs font-medium text-stone-600 dark:text-neutral-300">
              Excludes (comma-separated substrings, optional)
            </span>
            <input
              type="text"
              value={newExcludes}
              onChange={e => setNewExcludes(e.target.value)}
              placeholder="drafts/, .secret"
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
              {creating ? 'Creating…' : 'Create vault'}
            </button>
          </div>
        </form>
      ) : null}

      {loading ? (
        <div className="py-4 text-center text-xs text-stone-400 dark:text-neutral-500">
          Loading vaults…
        </div>
      ) : loadError ? (
        <div className="rounded border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-xs text-coral-800">
          Failed to load vaults: {loadError}
        </div>
      ) : vaults.length === 0 ? (
        <div className="py-4 text-center text-xs text-stone-400 dark:text-neutral-500">
          No vaults yet. Add one above to start ingesting a folder.
        </div>
      ) : (
        <ul className="divide-y divide-stone-100 dark:divide-neutral-800" data-testid="vault-list">
          {vaults.map(v => {
            const state = busy[v.id];
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
                    {v.file_count.toLocaleString()} file(s) ·{' '}
                    {v.last_synced_at
                      ? `synced ${formatRelative(v.last_synced_at)}`
                      : 'never synced'}
                  </div>
                </div>
                <div className="flex items-center gap-2">
                  <button
                    type="button"
                    onClick={() => void handleSync(v)}
                    disabled={state === 'sync' || state === 'remove'}
                    className="rounded-md border border-primary-300 bg-white dark:bg-neutral-900 px-3 py-1.5 text-xs
                               font-semibold text-primary-700 dark:text-primary-300 shadow-sm transition-colors
                               hover:bg-primary-50 dark:hover:bg-primary-500/15 disabled:cursor-not-allowed disabled:opacity-50">
                    {state === 'sync' ? 'Syncing…' : 'Sync'}
                  </button>
                  <button
                    type="button"
                    onClick={() => void handleRemove(v)}
                    disabled={state === 'sync' || state === 'remove'}
                    className="rounded-md border border-coral-200 dark:border-coral-500/30 bg-white dark:bg-neutral-900 px-3 py-1.5 text-xs
                               font-semibold text-coral-700 dark:text-coral-300 shadow-sm transition-colors
                               hover:bg-coral-50 dark:hover:bg-coral-500/10 disabled:cursor-not-allowed disabled:opacity-50">
                    {state === 'remove' ? 'Removing…' : 'Remove'}
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

function formatRelative(iso: string): string {
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return iso;
  const diff = Math.max(0, Date.now() - t);
  const sec = Math.floor(diff / 1000);
  if (sec < 60) return `${sec}s ago`;
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.floor(hr / 24);
  return `${day}d ago`;
}
