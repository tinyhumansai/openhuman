import { useCallback, useEffect, useMemo, useState } from 'react';

import type { ToastNotification } from '../../types/intelligence';
import { openUrl, revealPath } from '../../utils/openUrl';
import { memoryTreeVaultHealthCheck, type VaultHealthCheck } from '../../utils/tauriCommands';

const OBSIDIAN_DOWNLOAD_URL = 'https://obsidian.md/download';

interface VaultHealthChecklistProps {
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
  title?: string;
}

function formatRelativeTime(ms: number): string {
  if (!ms || ms <= 0) return 'Never';
  const diff = Date.now() - ms;
  if (diff < 0) return 'Never';
  const sec = Math.floor(diff / 1000);
  if (sec < 45) return 'just now';
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min} min ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr} hr ago`;
  const day = Math.floor(hr / 24);
  return `${day} day${day === 1 ? '' : 's'} ago`;
}

function dirname(path: string): string {
  const normalized = path.replace(/[\\/]+$/, '');
  const slash = Math.max(normalized.lastIndexOf('/'), normalized.lastIndexOf('\\'));
  if (slash <= 0) return normalized;
  return normalized.slice(0, slash);
}

export function VaultHealthChecklist({
  onToast,
  title = 'Vault Health Checklist',
}: VaultHealthChecklistProps) {
  const [health, setHealth] = useState<VaultHealthCheck | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const runCheck = useCallback(async () => {
    setRefreshing(true);
    try {
      const next = await memoryTreeVaultHealthCheck();
      setHealth(next);
      setError(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
    } finally {
      setRefreshing(false);
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void runCheck();
  }, [runCheck]);

  const revealTarget = useMemo(() => {
    if (!health?.content_root_abs) return '';
    return health.exists ? health.content_root_abs : dirname(health.content_root_abs);
  }, [health]);

  const openObsidian = useCallback(() => {
    if (!health?.content_root_abs) return;
    void (async () => {
      try {
        await openUrl(`obsidian://open?path=${encodeURIComponent(health.content_root_abs)}`);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        onToast?.({ type: 'error', title: 'Could not open Obsidian', message });
      }
    })();
  }, [health, onToast]);

  const revealVault = useCallback(() => {
    if (!revealTarget) return;
    void (async () => {
      try {
        await revealPath(revealTarget);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        onToast?.({ type: 'error', title: 'Could not reveal vault folder', message });
      }
    })();
  }, [onToast, revealTarget]);

  const installObsidian = useCallback(() => {
    void (async () => {
      try {
        await openUrl(OBSIDIAN_DOWNLOAD_URL);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        onToast?.({ type: 'error', title: 'Could not open Obsidian download page', message });
      }
    })();
  }, [onToast]);

  const checklist = health
    ? [
        {
          key: 'exists',
          label: 'Workspace vault path exists',
          ok: health.exists,
          recovery:
            'Vault folder is missing. Start a sync or create this folder, then refresh this checklist.',
        },
        {
          key: 'writable',
          label: 'Vault is writable by OpenHuman',
          ok: health.writable,
          recovery:
            'OpenHuman cannot write to this vault yet. Grant write permissions and refresh.',
        },
        {
          key: 'obsidian',
          label: 'Vault is registered in Obsidian',
          ok: health.obsidian_registered,
          recovery:
            'In Obsidian, choose "Open folder as vault" for this path, then refresh this checklist.',
        },
        {
          key: 'pipeline',
          label: 'Memory pipeline is healthy',
          ok: health.pipeline_healthy,
          recovery:
            'Memory pipeline is paused or in error. Re-enable Auto-sync in Memory Tree status and retry.',
        },
      ]
    : [];

  return (
    <div
      className="rounded-xl border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 p-4 space-y-3"
      data-testid="vault-health-checklist">
      <div className="flex items-start justify-between gap-2">
        <div>
          <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">{title}</h3>
          <p className="mt-1 text-xs text-stone-600 dark:text-neutral-300">
            Workspace vault: <code className="font-mono">memory_tree/content</code>
          </p>
        </div>
        <button
          type="button"
          onClick={() => {
            void runCheck();
          }}
          disabled={refreshing}
          className="rounded-md border border-stone-300 dark:border-neutral-600 bg-white dark:bg-neutral-800 px-2.5 py-1 text-xs font-medium text-stone-700 dark:text-neutral-200 disabled:opacity-60"
          data-testid="vault-health-refresh">
          {refreshing ? 'Refreshing…' : 'Refresh'}
        </button>
      </div>

      {health?.content_root_abs ? (
        <code
          className="block break-all rounded-md bg-stone-100 dark:bg-neutral-800 px-2 py-1 text-[11px] text-stone-700 dark:text-neutral-200"
          data-testid="vault-health-path">
          {health.content_root_abs}
        </code>
      ) : null}

      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          onClick={revealVault}
          disabled={!health?.content_root_abs}
          className="rounded-md border border-stone-300 dark:border-neutral-600 bg-white dark:bg-neutral-800 px-3 py-1.5 text-xs font-semibold text-stone-700 dark:text-neutral-200 disabled:opacity-50"
          data-testid="vault-health-reveal">
          Reveal Folder
        </button>
        <button
          type="button"
          onClick={openObsidian}
          disabled={!health?.content_root_abs}
          className="rounded-md border border-violet-300 dark:border-violet-500/40 bg-white dark:bg-neutral-800 px-3 py-1.5 text-xs font-semibold text-violet-700 dark:text-violet-300 disabled:opacity-50"
          data-testid="vault-health-open-obsidian">
          Open in Obsidian
        </button>
        <button
          type="button"
          onClick={installObsidian}
          className="rounded-md border border-stone-300 dark:border-neutral-600 bg-white dark:bg-neutral-800 px-3 py-1.5 text-xs font-semibold text-stone-700 dark:text-neutral-200"
          data-testid="vault-health-install-obsidian">
          Install Obsidian
        </button>
      </div>

      {loading ? (
        <div className="h-16 rounded-md bg-stone-100 dark:bg-neutral-800 animate-pulse" />
      ) : error ? (
        <div
          className="rounded-md border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-xs text-coral-700 dark:text-coral-300"
          data-testid="vault-health-error">
          Could not load vault health: {error}
        </div>
      ) : (
        <div className="space-y-2">
          {checklist.map(item => (
            <div
              key={item.key}
              data-testid={`vault-health-item-${item.key}`}
              className={`rounded-md border px-3 py-2 text-xs ${
                item.ok
                  ? 'border-sage-200 dark:border-sage-500/30 bg-sage-50 dark:bg-sage-500/10 text-sage-800 dark:text-sage-200'
                  : 'border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 text-amber-800 dark:text-amber-200'
              }`}>
              <div className="font-semibold">
                {item.ok ? 'Passed' : 'Needs attention'} · {item.label}
              </div>
              {!item.ok ? <p className="mt-1 leading-relaxed">{item.recovery}</p> : null}
            </div>
          ))}
          <p
            className="text-[11px] text-stone-600 dark:text-neutral-300"
            data-testid="vault-health-last-sync">
            Last sync: {formatRelativeTime(health?.last_sync_ms ?? 0)}
          </p>
        </div>
      )}
    </div>
  );
}

export default VaultHealthChecklist;
