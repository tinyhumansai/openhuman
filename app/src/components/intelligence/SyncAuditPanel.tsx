/**
 * Sync audit history panel — shows when syncs happened, tokens consumed,
 * cost, and duration. Fetches from `openhuman.memory_sources_sync_audit_log`.
 */
import { useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { memorySyncAuditLog, type SyncAuditEntry } from '../../utils/tauriCommands';

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const secs = ms / 1000;
  if (secs < 60) return `${secs.toFixed(1)}s`;
  const mins = Math.floor(secs / 60);
  const remSecs = Math.round(secs % 60);
  return `${mins}m ${remSecs}s`;
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

function scopeLabel(scope: string): string {
  if (scope.startsWith('github:')) {
    return `GitHub · ${scope.slice(7)}`;
  }
  if (scope.startsWith('gmail:')) {
    return `Gmail · ${scope.slice(6).replace(/-at-/g, '@').replace(/-dot-/g, '.')}`;
  }
  if (scope.startsWith('rebuild:')) {
    return `Rebuild · ${scope.slice(8)}`;
  }
  return scope;
}

// `t` is threaded in because this is a module-level helper with no hook scope.
// The `{n}` placeholder follows the codebase's interpolation convention
// (t(...).replace('{n}', value)) — `t()` itself does not interpolate params.
export function timeAgo(iso: string, t: (key: string, fallback?: string) => string): string {
  const diff = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diff / 60_000);
  if (mins < 1) return t('sync.timeAgo.justNow', 'just now');
  if (mins < 60) return t('sync.timeAgo.minutes', '{n}m ago').replace('{n}', String(mins));
  const hours = Math.floor(mins / 60);
  if (hours < 24) return t('sync.timeAgo.hours', '{n}h ago').replace('{n}', String(hours));
  const days = Math.floor(hours / 24);
  return t('sync.timeAgo.days', '{n}d ago').replace('{n}', String(days));
}

export function SyncAuditPanel() {
  const { t } = useT();
  const [entries, setEntries] = useState<SyncAuditEntry[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const data = await memorySyncAuditLog();
        if (!cancelled) setEntries(data);
      } catch (err) {
        console.error('[sync-audit] fetch failed', err);
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  if (loading) {
    return (
      <div className="text-xs text-content-faint py-2">{t('common.loading', 'Loading...')}</div>
    );
  }

  if (entries.length === 0) {
    return (
      <div className="text-xs text-content-faint py-2">
        {t('sync.noAuditEntries', 'No sync runs recorded yet.')}
      </div>
    );
  }

  const totalCost = entries.reduce((s, e) => s + e.estimated_cost_usd, 0);
  const totalInput = entries.reduce((s, e) => s + e.input_tokens, 0);
  const totalOutput = entries.reduce((s, e) => s + e.output_tokens, 0);

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-4 text-xs text-content-muted">
        <span>
          {entries.length} {t('sync.runs', 'sync runs')}
        </span>
        <span className="text-content-faint dark:text-neutral-600">·</span>
        <span>
          {formatTokens(totalInput)} in / {formatTokens(totalOutput)} out
        </span>
        <span className="text-content-faint dark:text-neutral-600">·</span>
        <span className="font-medium">
          ${totalCost.toFixed(4)} {t('sync.totalCost', 'total')}
        </span>
      </div>
      <div className="max-h-48 overflow-y-auto rounded-md border border-line-subtle">
        <table className="w-full text-xs">
          <thead className="sticky top-0 bg-surface-muted text-content-muted">
            <tr>
              <th className="px-3 py-1.5 text-left font-medium">{t('sync.when', 'When')}</th>
              <th className="px-3 py-1.5 text-left font-medium">{t('sync.source', 'Source')}</th>
              <th className="px-3 py-1.5 text-right font-medium">{t('sync.items', 'Items')}</th>
              <th className="px-3 py-1.5 text-right font-medium">{t('sync.tokens', 'Tokens')}</th>
              <th className="px-3 py-1.5 text-right font-medium">{t('sync.cost', 'Cost')}</th>
              <th className="px-3 py-1.5 text-right font-medium">
                {t('sync.duration', 'Duration')}
              </th>
              <th className="px-3 py-1.5 text-center font-medium"></th>
            </tr>
          </thead>
          <tbody className="divide-y divide-stone-50 dark:divide-neutral-800">
            {entries.map((e, i) => (
              <tr
                key={`${e.timestamp}-${i}`}
                className="hover:bg-surface-muted dark:hover:bg-surface-muted/50">
                <td
                  className="px-3 py-1.5 text-content-secondary whitespace-nowrap"
                  title={e.timestamp}>
                  {timeAgo(e.timestamp, t)}
                </td>
                <td
                  className="px-3 py-1.5 text-content-secondary truncate max-w-[180px]"
                  title={e.scope}>
                  {scopeLabel(e.scope)}
                </td>
                <td className="px-3 py-1.5 text-right tabular-nums text-content-secondary">
                  {e.items_fetched}
                </td>
                <td
                  className="px-3 py-1.5 text-right tabular-nums text-content-secondary"
                  title={`${e.input_tokens} in / ${e.output_tokens} out`}>
                  {formatTokens(e.input_tokens + e.output_tokens)}
                </td>
                <td className="px-3 py-1.5 text-right tabular-nums font-medium text-content-secondary">
                  ${e.estimated_cost_usd.toFixed(4)}
                </td>
                <td className="px-3 py-1.5 text-right tabular-nums text-content-muted">
                  {formatDuration(e.duration_ms)}
                </td>
                <td className="px-3 py-1.5 text-center">
                  {e.success ? (
                    <span className="text-green-500" title={t('sync.status.success', 'Success')}>
                      ✓
                    </span>
                  ) : (
                    <span
                      className="text-red-500"
                      title={e.error ?? t('sync.status.failed', 'Failed')}>
                      ✗
                    </span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
