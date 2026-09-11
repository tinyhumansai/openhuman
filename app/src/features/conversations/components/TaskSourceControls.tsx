import createDebug from 'debug';
import { useCallback, useEffect, useState } from 'react';
import { LuRefreshCw } from 'react-icons/lu';
import { useNavigate } from 'react-router-dom';

import { Alert } from '../../../components/ui/Alert';
import Badge from '../../../components/ui/Badge';
import Button from '../../../components/ui/Button';
import Card from '../../../components/ui/Card';
import { useT } from '../../../lib/i18n/I18nContext';
import {
  isTauri,
  openhumanTaskSourcesFetch,
  openhumanTaskSourcesList,
  openhumanTaskSourcesStatus,
  openhumanTaskSourcesSync,
  openhumanTaskSourcesUpdate,
  type TaskSource,
  type TaskSourcesStatus,
} from '../../../utils/tauriCommands';
import { formatFetchNotice, formatSyncNotice, providerLabel } from './taskBoardMetadata';

const log = createDebug('app:conversations:task-sources');

/**
 * The task-source management strip that folds out of the board header.
 *
 * Extracted out of `TaskKanbanBoard.tsx` when that file was split. The panel
 * chrome is the shared {@link Card}, the error/notice lines are {@link Alert}s
 * (which carry `role="alert"`/`data-variant` rather than a tinted `<p>`), the
 * per-source enabled state is a {@link Badge}, and every control is the shared
 * {@link Button} — the previous `text-ocean-600` link colour named a scale that
 * does not exist in `tailwind.config.js` and therefore emitted no CSS.
 */
export function TaskSourceControls({ disabled, compact }: { disabled: boolean; compact: boolean }) {
  const { t } = useT();
  const navigate = useNavigate();
  const [loading, setLoading] = useState(true);
  const [sources, setSources] = useState<TaskSource[]>([]);
  const [status, setStatus] = useState<TaskSourcesStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!isTauri()) {
      log('load skipped: not running under Tauri');
      setLoading(false);
      setError(t('conversations.taskKanban.sources.desktopOnly'));
      return;
    }
    log('load start');
    setLoading(true);
    setError(null);
    try {
      const [nextSources, nextStatus] = await Promise.all([
        openhumanTaskSourcesList(),
        openhumanTaskSourcesStatus(),
      ]);
      log('load ok sources=%d enabled=%s', nextSources.length, nextStatus.enabled);
      setSources(nextSources);
      setStatus(nextStatus);
    } catch (err) {
      log('load failed err=%o', err);
      setError(
        `${t('settings.taskSources.loadError')}: ${err instanceof Error ? err.message : String(err)}`
      );
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    const id = window.setTimeout(() => {
      void load();
    }, 0);
    return () => window.clearTimeout(id);
  }, [load]);

  const toggleSource = async (source: TaskSource) => {
    if (busyKey) return;
    log('toggle source=%s -> enabled=%s', source.id, !source.enabled);
    setBusyKey(`toggle:${source.id}`);
    setError(null);
    setNotice(null);
    try {
      const updated = await openhumanTaskSourcesUpdate(source.id, { enabled: !source.enabled });
      setSources(prev => prev.map(item => (item.id === updated.id ? updated : item)));
    } catch (err) {
      log('toggle failed source=%s err=%o', source.id, err);
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyKey(null);
    }
  };

  const fetchSource = async (source: TaskSource) => {
    if (busyKey) return;
    log('fetch source=%s', source.id);
    setBusyKey(`fetch:${source.id}`);
    setError(null);
    setNotice(null);
    try {
      const outcome = await openhumanTaskSourcesFetch(source.id);
      await load();
      if (outcome.error) {
        log('fetch source=%s returned an error outcome', source.id);
        setError(outcome.error);
      } else {
        setNotice(formatFetchNotice(outcome, t));
      }
    } catch (err) {
      log('fetch failed source=%s err=%o', source.id, err);
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyKey(null);
    }
  };

  const syncSources = async () => {
    if (busyKey) return;
    log('sync all sources');
    setBusyKey('sync');
    setError(null);
    setNotice(null);
    try {
      const outcomes = await openhumanTaskSourcesSync();
      await load();
      const firstError = outcomes.find(outcome => outcome.error)?.error;
      if (firstError) {
        log('sync returned an error outcome');
        setError(firstError);
      } else {
        setNotice(formatSyncNotice(outcomes, t));
      }
    } catch (err) {
      log('sync failed err=%o', err);
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyKey(null);
    }
  };

  return (
    <Card className="mb-3">
      <div className="p-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="min-w-0">
            <h5 className="text-xs font-semibold text-content">
              {t('conversations.taskKanban.sources.title')}
            </h5>
            {!compact && status && (
              <p className="text-[11px] text-content-muted">
                {status.enabled
                  ? t('conversations.taskKanban.sources.statusEnabled')
                  : t('settings.taskSources.disabledBanner')}
              </p>
            )}
          </div>
          <div className="flex items-center gap-2">
            <Button
              variant="tertiary"
              size="xs"
              onClick={() => navigate('/settings/integrations')}
              className="px-0 text-[11px] text-primary-600 hover:bg-transparent hover:underline dark:text-primary-300">
              {t('conversations.taskKanban.sources.manage')}
            </Button>
            <Button
              variant="secondary"
              size="xs"
              disabled={disabled || loading || busyKey !== null || sources.length === 0}
              onClick={() => void syncSources()}
              className="gap-1 px-2 text-[11px]">
              <LuRefreshCw className="h-3 w-3" />
              {busyKey === 'sync'
                ? t('settings.taskSources.syncing')
                : t('settings.taskSources.syncAll')}
            </Button>
            <Button
              iconOnly
              variant="secondary"
              size="sm"
              aria-label={t('settings.taskSources.refresh')}
              disabled={loading}
              onClick={() => void load()}
              className="h-7 w-7">
              <LuRefreshCw className="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>
        {error && (
          <Alert variant="destructive" className="mt-2 text-[11px]">
            {error}
          </Alert>
        )}
        {notice && (
          <Alert variant="info" className="mt-2 text-[11px]">
            {notice}
          </Alert>
        )}
        {loading ? (
          <p className="mt-2 text-[11px] text-content-faint">{t('common.loading')}</p>
        ) : sources.length === 0 ? (
          <p className="mt-2 text-[11px] text-content-faint">{t('settings.taskSources.empty')}</p>
        ) : (
          <ul className="mt-3 grid gap-2 sm:grid-cols-2">
            {sources.map(source => (
              <li key={source.id} className="min-w-0 rounded-lg border border-line px-2.5 py-2">
                <div className="flex items-start justify-between gap-2">
                  <div className="min-w-0">
                    <p className="truncate text-xs font-medium text-content">
                      {source.name || providerLabel(source.provider, t)}
                    </p>
                    <p className="truncate text-[11px] text-content-muted">
                      {providerLabel(source.provider, t)}
                      {source.target === 'agent_todo_proactive'
                        ? ` · ${t('settings.taskSources.proactive')}`
                        : ''}
                    </p>
                  </div>
                  <Badge
                    variant={source.enabled ? 'success' : 'neutral'}
                    className="flex-none text-[10px]">
                    {source.enabled
                      ? t('settings.taskSources.statusEnabled')
                      : t('settings.taskSources.statusDisabled')}
                  </Badge>
                </div>
                <div className="mt-2 flex flex-wrap gap-1.5">
                  <Button
                    variant="secondary"
                    size="xs"
                    disabled={disabled || busyKey !== null}
                    onClick={() => void fetchSource(source)}
                    className="gap-1 px-2 text-[11px]">
                    <LuRefreshCw className="h-3 w-3" />
                    {busyKey === `fetch:${source.id}`
                      ? t('settings.taskSources.fetching')
                      : t('settings.taskSources.fetchNow')}
                  </Button>
                  <Button
                    variant="secondary"
                    size="xs"
                    disabled={disabled || busyKey !== null}
                    onClick={() => void toggleSource(source)}
                    className="px-2 text-[11px]">
                    {source.enabled
                      ? t('settings.taskSources.disable')
                      : t('settings.taskSources.enable')}
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </Card>
  );
}
