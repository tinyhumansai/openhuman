import { useCallback, useEffect, useMemo, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  openhumanTaskSourcesAdd,
  openhumanTaskSourcesFetch,
  openhumanTaskSourcesList,
  openhumanTaskSourcesPreviewFilter,
  openhumanTaskSourcesRemove,
  openhumanTaskSourcesStatus,
  openhumanTaskSourcesUpdate,
  type TaskSource,
  type TaskSourceFilter,
  type TaskSourceProvider,
  type TaskSourcesStatus,
} from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const PROVIDERS: TaskSourceProvider[] = ['github', 'notion', 'linear', 'clickup'];

function providerLabel(provider: TaskSourceProvider, t: (key: string) => string): string {
  switch (provider) {
    case 'github':
      return t('settings.taskSources.providers.github');
    case 'notion':
      return t('settings.taskSources.providers.notion');
    case 'linear':
      return t('settings.taskSources.providers.linear');
    case 'clickup':
      return t('settings.taskSources.providers.clickup');
    default:
      return provider;
  }
}

/** Build a `TaskSourceFilter` from the create-form fields. */
function buildFilter(
  provider: TaskSourceProvider,
  fields: { primary: string; labels: string; assignedToMe: boolean }
): TaskSourceFilter {
  const primary = fields.primary.trim();
  switch (provider) {
    case 'github':
      return {
        provider: 'github',
        repo: primary || undefined,
        labels: fields.labels
          .split(',')
          .map(l => l.trim())
          .filter(Boolean),
        assignee_is_me: fields.assignedToMe,
      };
    case 'notion':
      return {
        provider: 'notion',
        database_id: primary || undefined,
        assigned_to_me: fields.assignedToMe,
      };
    case 'linear':
      return {
        provider: 'linear',
        team_id: primary || undefined,
        assignee_is_me: fields.assignedToMe,
      };
    case 'clickup':
      return {
        provider: 'clickup',
        team_id: primary || undefined,
        assignee_is_me: fields.assignedToMe,
      };
    default:
      return { provider: 'github', assignee_is_me: fields.assignedToMe };
  }
}

const TaskSourcesPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [sources, setSources] = useState<TaskSource[]>([]);
  const [status, setStatus] = useState<TaskSourcesStatus | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  // ── create-form state ────────────────────────────────────────────
  const [provider, setProvider] = useState<TaskSourceProvider>('github');
  const [name, setName] = useState('');
  const [primary, setPrimary] = useState('');
  const [labels, setLabels] = useState('');
  const [assignedToMe, setAssignedToMe] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [list, stat] = await Promise.all([
        openhumanTaskSourcesList(),
        openhumanTaskSourcesStatus(),
      ]);
      setSources(list);
      setStatus(stat);
    } catch (err) {
      setError(
        `${t('settings.taskSources.loadError')}: ${err instanceof Error ? err.message : String(err)}`
      );
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    void load();
  }, [load]);

  const primaryLabel = useMemo(() => {
    switch (provider) {
      case 'github':
        return t('settings.taskSources.github.repo');
      case 'notion':
        return t('settings.taskSources.notion.database');
      case 'linear':
        return t('settings.taskSources.linear.team');
      case 'clickup':
        return t('settings.taskSources.clickup.team');
      default:
        return '';
    }
  }, [provider, t]);

  const addSource = async () => {
    setBusyKey('add');
    setError(null);
    setNotice(null);
    try {
      await openhumanTaskSourcesAdd({
        provider,
        name: name.trim() || undefined,
        filter: buildFilter(provider, { primary, labels, assignedToMe }),
      });
      setName('');
      setPrimary('');
      setLabels('');
      await load();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyKey(null);
    }
  };

  const previewFilter = async () => {
    setBusyKey('preview');
    setError(null);
    setNotice(null);
    try {
      const tasks = await openhumanTaskSourcesPreviewFilter(
        provider,
        buildFilter(provider, { primary, labels, assignedToMe })
      );
      setNotice(t('settings.taskSources.previewResult').replace('{count}', String(tasks.length)));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyKey(null);
    }
  };

  const toggleSource = async (source: TaskSource) => {
    setBusyKey(`toggle:${source.id}`);
    setError(null);
    try {
      const updated = await openhumanTaskSourcesUpdate(source.id, { enabled: !source.enabled });
      setSources(prev => prev.map(s => (s.id === updated.id ? updated : s)));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyKey(null);
    }
  };

  const fetchNow = async (source: TaskSource) => {
    setBusyKey(`fetch:${source.id}`);
    setError(null);
    setNotice(null);
    try {
      const outcome = await openhumanTaskSourcesFetch(source.id);
      // Refresh the source list first (updates lastFetchAt/lastStatus);
      // `load()` resets the error/notice, so set the outcome message
      // *after* it so the message isn't immediately cleared.
      await load();
      if (outcome.error) {
        setError(outcome.error);
      } else {
        setNotice(
          t('settings.taskSources.fetchResult')
            .replace('{routed}', String(outcome.routed))
            .replace('{fetched}', String(outcome.fetched))
        );
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyKey(null);
    }
  };

  const removeSource = async (source: TaskSource) => {
    if (!window.confirm(t('settings.taskSources.removeConfirm'))) return;
    setBusyKey(`remove:${source.id}`);
    setError(null);
    try {
      await openhumanTaskSourcesRemove(source.id);
      setSources(prev => prev.filter(s => s.id !== source.id));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyKey(null);
    }
  };

  return (
    <div data-testid="task-sources-panel">
      <SettingsHeader
        title={t('settings.taskSources.title')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-5">
        <section className="space-y-1">
          <p className="text-xs text-stone-500 dark:text-neutral-400">
            {t('settings.taskSources.description')}
          </p>
          <p className="text-xs text-stone-400 dark:text-neutral-500">
            {t('settings.taskSources.connectHint')}
          </p>
        </section>

        {status && !status.enabled && (
          <div className="rounded-lg border border-amber-300 dark:border-amber-500/40 bg-amber-50 dark:bg-amber-500/10 px-4 py-3 text-sm text-amber-700 dark:text-amber-300">
            {t('settings.taskSources.disabledBanner')}
          </div>
        )}
        {error && (
          <div className="rounded-lg border border-red-300 dark:border-red-500/40 bg-red-50 dark:bg-red-500/10 px-4 py-3 text-sm text-red-700 dark:text-red-300">
            {error}
          </div>
        )}
        {notice && (
          <div className="rounded-lg border border-sky-300 dark:border-sky-500/40 bg-sky-50 dark:bg-sky-500/10 px-4 py-3 text-sm text-sky-700 dark:text-sky-300">
            {notice}
          </div>
        )}

        {/* ── Add a source ─────────────────────────────────────────── */}
        <section className="rounded-xl border border-stone-200 dark:border-neutral-800 p-4 space-y-3">
          <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
            {t('settings.taskSources.addTitle')}
          </h3>

          <label className="block text-xs text-stone-500 dark:text-neutral-400">
            {t('settings.taskSources.provider')}
            <select
              className="mt-1 w-full rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100"
              value={provider}
              onChange={e => setProvider(e.target.value as TaskSourceProvider)}>
              {PROVIDERS.map(p => (
                <option key={p} value={p}>
                  {providerLabel(p, t)}
                </option>
              ))}
            </select>
          </label>

          <label className="block text-xs text-stone-500 dark:text-neutral-400">
            {t('settings.taskSources.name')}
            <input
              type="text"
              className="mt-1 w-full rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100"
              placeholder={t('settings.taskSources.namePlaceholder')}
              value={name}
              onChange={e => setName(e.target.value)}
            />
          </label>

          <label className="block text-xs text-stone-500 dark:text-neutral-400">
            {primaryLabel}
            <input
              type="text"
              className="mt-1 w-full rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100"
              value={primary}
              onChange={e => setPrimary(e.target.value)}
            />
          </label>

          {provider === 'github' && (
            <label className="block text-xs text-stone-500 dark:text-neutral-400">
              {t('settings.taskSources.github.labels')}
              <input
                type="text"
                className="mt-1 w-full rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100"
                value={labels}
                onChange={e => setLabels(e.target.value)}
              />
            </label>
          )}

          <label className="flex items-center gap-2 text-xs text-stone-600 dark:text-neutral-300">
            <input
              type="checkbox"
              checked={assignedToMe}
              onChange={e => setAssignedToMe(e.target.checked)}
            />
            {t('settings.taskSources.assignedToMe')}
          </label>

          <div className="flex gap-2 pt-1">
            <button
              type="button"
              className="btn btn-primary btn-sm"
              disabled={busyKey === 'add'}
              onClick={() => void addSource()}>
              {busyKey === 'add' ? t('settings.taskSources.adding') : t('settings.taskSources.add')}
            </button>
            <button
              type="button"
              className="btn btn-outline btn-sm"
              disabled={busyKey === 'preview'}
              onClick={() => void previewFilter()}>
              {t('settings.taskSources.preview')}
            </button>
          </div>
        </section>

        {/* ── Configured sources ───────────────────────────────────── */}
        <section className="space-y-2">
          <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
            {t('settings.taskSources.configured')}
          </h3>

          {loading ? (
            <p className="text-sm text-stone-400 dark:text-neutral-500">{t('common.loading')}</p>
          ) : sources.length === 0 ? (
            <p className="text-sm text-stone-400 dark:text-neutral-500">
              {t('settings.taskSources.empty')}
            </p>
          ) : (
            <ul className="space-y-2">
              {sources.map(source => (
                <li
                  key={source.id}
                  className="rounded-lg border border-stone-200 dark:border-neutral-800 p-3 space-y-2"
                  data-testid={`task-source-${source.id}`}>
                  <div className="flex items-start justify-between gap-2">
                    <div>
                      <p className="text-sm font-medium text-stone-900 dark:text-neutral-100">
                        {source.name || providerLabel(source.provider, t)}
                      </p>
                      <p className="text-xs text-stone-400 dark:text-neutral-500">
                        {providerLabel(source.provider, t)}
                        {source.target === 'agent_todo_proactive'
                          ? ` · ${t('settings.taskSources.proactive')}`
                          : ''}
                      </p>
                      <p className="text-xs text-stone-400 dark:text-neutral-500">
                        {t('settings.taskSources.lastFetch')}:{' '}
                        {source.lastFetchAt
                          ? new Date(source.lastFetchAt).toLocaleString()
                          : t('settings.taskSources.never')}
                      </p>
                    </div>
                    <span
                      className={`text-xs rounded-full px-2 py-0.5 ${
                        source.enabled
                          ? 'bg-sage-100 text-sage-700 dark:bg-sage-500/15 dark:text-sage-300'
                          : 'bg-stone-100 text-stone-500 dark:bg-neutral-800 dark:text-neutral-400'
                      }`}>
                      {source.enabled
                        ? t('settings.taskSources.statusEnabled')
                        : t('settings.taskSources.statusDisabled')}
                    </span>
                  </div>

                  <div className="flex flex-wrap gap-2">
                    <button
                      type="button"
                      className="btn btn-outline btn-xs"
                      disabled={busyKey === `toggle:${source.id}`}
                      onClick={() => void toggleSource(source)}>
                      {source.enabled
                        ? t('settings.taskSources.disable')
                        : t('settings.taskSources.enable')}
                    </button>
                    <button
                      type="button"
                      className="btn btn-outline btn-xs"
                      disabled={busyKey === `fetch:${source.id}`}
                      onClick={() => void fetchNow(source)}>
                      {busyKey === `fetch:${source.id}`
                        ? t('settings.taskSources.fetching')
                        : t('settings.taskSources.fetchNow')}
                    </button>
                    <button
                      type="button"
                      className="btn btn-ghost btn-xs text-red-600 dark:text-red-400"
                      disabled={busyKey === `remove:${source.id}`}
                      onClick={() => void removeSource(source)}>
                      {t('settings.taskSources.remove')}
                    </button>
                  </div>
                </li>
              ))}
            </ul>
          )}

          <button type="button" className="btn btn-ghost btn-sm" onClick={() => void load()}>
            {t('settings.taskSources.refresh')}
          </button>
        </section>
      </div>
    </div>
  );
};

export default TaskSourcesPanel;
