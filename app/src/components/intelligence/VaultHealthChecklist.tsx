import { useCallback, useEffect, useMemo, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { ToastNotification } from '../../types/intelligence';
import { openUrl, revealPath } from '../../utils/openUrl';
import { memoryTreeVaultHealthCheck, type VaultHealthCheck } from '../../utils/tauriCommands';
import Button from '../ui/Button';

const OBSIDIAN_DOWNLOAD_URL = 'https://obsidian.md/download';

interface VaultHealthChecklistProps {
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
  title?: string;
}

function formatRelativeTime(ms: number, t: (key: string, fallback?: string) => string): string {
  if (!ms || ms <= 0) return t('vaultHealth.timeNever');
  const diff = Date.now() - ms;
  if (diff < 0) return t('vaultHealth.timeNever');
  const sec = Math.floor(diff / 1000);
  if (sec < 45) return t('vaultHealth.timeJustNow');
  const min = Math.floor(sec / 60);
  if (min < 60) return t('vaultHealth.timeMinAgo').replace('{n}', String(min));
  const hr = Math.floor(min / 60);
  if (hr < 24) return t('vaultHealth.timeHrAgo').replace('{n}', String(hr));
  const day = Math.floor(hr / 24);
  return (day === 1 ? t('vaultHealth.timeDayAgo') : t('vaultHealth.timeDaysAgo')).replace(
    '{n}',
    String(day)
  );
}

function dirname(path: string): string {
  const normalized = path.replace(/[\\/]+$/, '');
  const slash = Math.max(normalized.lastIndexOf('/'), normalized.lastIndexOf('\\'));
  if (slash <= 0) return normalized;
  return normalized.slice(0, slash);
}

export function VaultHealthChecklist({ onToast, title }: VaultHealthChecklistProps) {
  const { t } = useT();
  const resolvedTitle = title ?? t('vaultHealth.title');
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
        onToast?.({ type: 'error', title: t('vaultHealth.openObsidianError'), message });
      }
    })();
  }, [health, onToast, t]);

  const revealVault = useCallback(() => {
    if (!revealTarget) return;
    void (async () => {
      try {
        await revealPath(revealTarget);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        onToast?.({ type: 'error', title: t('vaultHealth.revealError'), message });
      }
    })();
  }, [onToast, revealTarget, t]);

  const installObsidian = useCallback(() => {
    void (async () => {
      try {
        await openUrl(OBSIDIAN_DOWNLOAD_URL);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        onToast?.({ type: 'error', title: t('vaultHealth.downloadError'), message });
      }
    })();
  }, [onToast, t]);

  const checklist = health
    ? [
        {
          key: 'exists',
          label: t('vaultHealth.existsLabel'),
          ok: health.exists,
          recovery: t('vaultHealth.existsRecovery'),
        },
        {
          key: 'writable',
          label: t('vaultHealth.writableLabel'),
          ok: health.writable,
          recovery: t('vaultHealth.writableRecovery'),
        },
        {
          key: 'obsidian',
          label: t('vaultHealth.obsidianLabel'),
          ok: health.obsidian_registered,
          recovery: t('vaultHealth.obsidianRecovery'),
        },
        {
          key: 'pipeline',
          label: t('vaultHealth.pipelineLabel'),
          ok: health.pipeline_healthy,
          recovery: t('vaultHealth.pipelineRecovery'),
        },
      ]
    : [];

  return (
    <div
      className="rounded-xl border border-line bg-surface p-4 space-y-3"
      data-testid="vault-health-checklist">
      <div className="flex items-start justify-between gap-2">
        <div>
          <h3 className="text-sm font-semibold text-content">{resolvedTitle}</h3>
          <p className="mt-1 text-xs text-content-secondary">
            {t('vaultHealth.workspaceVault')} <code className="font-mono">memory_tree/content</code>
          </p>
        </div>
        <Button
          variant="secondary"
          size="xs"
          onClick={() => {
            void runCheck();
          }}
          disabled={refreshing}
          data-testid="vault-health-refresh">
          {refreshing ? t('vaultHealth.refreshing') : t('vaultHealth.refresh')}
        </Button>
      </div>

      {health?.content_root_abs ? (
        <code
          className="block break-all rounded-md bg-surface-subtle px-2 py-1 text-[11px] text-content-secondary"
          data-testid="vault-health-path">
          {health.content_root_abs}
        </code>
      ) : null}

      <div className="flex flex-wrap gap-2">
        <Button
          variant="secondary"
          size="sm"
          onClick={revealVault}
          disabled={!health?.content_root_abs}
          data-testid="vault-health-reveal">
          {t('vaultHealth.revealFolder')}
        </Button>
        <button
          type="button"
          onClick={openObsidian}
          disabled={!health?.content_root_abs}
          className="rounded-md border border-violet-300 dark:border-violet-500/40 bg-surface px-3 py-1.5 text-xs font-semibold text-violet-700 dark:text-violet-300 disabled:opacity-50"
          data-testid="vault-health-open-obsidian">
          {t('vaultHealth.openInObsidian')}
        </button>
        <Button
          variant="secondary"
          size="sm"
          onClick={installObsidian}
          data-testid="vault-health-install-obsidian">
          {t('vaultHealth.installObsidian')}
        </Button>
      </div>

      {loading ? (
        <div className="h-16 rounded-md bg-surface-subtle animate-pulse" />
      ) : error ? (
        <div
          className="rounded-md border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-xs text-coral-700 dark:text-coral-300"
          data-testid="vault-health-error">
          {t('vaultHealth.loadError')} {error}
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
                {item.ok ? t('vaultHealth.passed') : t('vaultHealth.needsAttention')} · {item.label}
              </div>
              {!item.ok ? <p className="mt-1 leading-relaxed">{item.recovery}</p> : null}
            </div>
          ))}
          <p className="text-[11px] text-content-secondary" data-testid="vault-health-last-sync">
            {t('vaultHealth.lastSync')} {formatRelativeTime(health?.last_sync_ms ?? 0, t)}
          </p>
        </div>
      )}
    </div>
  );
}

export default VaultHealthChecklist;
