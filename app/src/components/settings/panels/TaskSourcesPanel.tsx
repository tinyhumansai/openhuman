import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  openhumanTaskSourcesAdd,
  openhumanTaskSourcesFetch,
  openhumanTaskSourcesList,
  openhumanTaskSourcesListDatabases,
  openhumanTaskSourcesPreviewFilter,
  openhumanTaskSourcesRemove,
  openhumanTaskSourcesStatus,
  openhumanTaskSourcesSync,
  openhumanTaskSourcesUpdate,
  type TaskContainer,
  type TaskSource,
  type TaskSourceFilter,
  type TaskSourceProvider,
  type TaskSourcesStatus,
} from '../../../utils/tauriCommands';
import Button from '../../ui/Button';
import {
  SettingsBadge,
  SettingsCheckbox,
  SettingsEmptyState,
  SettingsRow,
  SettingsSection,
  SettingsSelect,
  SettingsStatusLine,
  SettingsTextField,
} from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

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

function formatSyncNotice(outcomes: Array<{ fetched: number; routed: number; pruned?: number }>): {
  fetched: number;
  routed: number;
  pruned: number;
} {
  return outcomes.reduce<{ fetched: number; routed: number; pruned: number }>(
    (totals, outcome) => ({
      fetched: totals.fetched + outcome.fetched,
      routed: totals.routed + outcome.routed,
      pruned: totals.pruned + (outcome.pruned ?? 0),
    }),
    { fetched: 0, routed: 0, pruned: 0 }
  );
}

interface TaskSourcesPanelProps {
  /** When true the panel is hosted inside another settings page (the
   *  Integrations tabs) — skip the standalone SettingsHeader chrome. */
  embedded?: boolean;
}

const TaskSourcesPanel = ({ embedded = false }: TaskSourcesPanelProps) => {
  const { t } = useT();

  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [sources, setSources] = useState<TaskSource[]>([]);
  const [status, setStatus] = useState<TaskSourcesStatus | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const loadingRef = useRef(loading);
  const busyKeyRef = useRef(busyKey);

  useEffect(() => {
    loadingRef.current = loading;
  }, [loading]);

  useEffect(() => {
    busyKeyRef.current = busyKey;
  }, [busyKey]);

  // ── create-form state ────────────────────────────────────────────
  const [provider, setProvider] = useState<TaskSourceProvider>('github');
  const [name, setName] = useState('');
  const [primary, setPrimary] = useState('');
  const [labels, setLabels] = useState('');
  const [assignedToMe, setAssignedToMe] = useState(true);
  // Notion database picker: populated on demand via `browseDatabases`.
  const [databases, setDatabases] = useState<TaskContainer[]>([]);

  // Clear any loaded database picker when the provider changes — the list is
  // provider-specific (today only Notion exposes one).
  useEffect(() => {
    setDatabases([]);
  }, [provider]);

  const load = useCallback(
    async (options?: { force?: boolean }) => {
      if (!options?.force && (loadingRef.current || busyKeyRef.current !== null)) return;
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
    },
    [t]
  );

  useEffect(() => {
    void load({ force: true });
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
    if (busyKey) return;
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
      await load({ force: true });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyKey(null);
    }
  };

  const previewFilter = async () => {
    if (busyKey) return;
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

  // Fetch the databases the connected account exposes (Notion) so the user can
  // pick one instead of pasting a raw id.
  const browseDatabases = async () => {
    if (busyKey) return;
    setBusyKey('databases');
    setError(null);
    setNotice(null);
    try {
      const dbs = await openhumanTaskSourcesListDatabases(provider);
      setDatabases(dbs);
      if (dbs.length === 0) {
        setNotice(t('settings.taskSources.notion.noDatabases'));
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyKey(null);
    }
  };

  const toggleSource = async (source: TaskSource) => {
    if (busyKey) return;
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
    if (busyKey) return;
    setBusyKey(`fetch:${source.id}`);
    setError(null);
    setNotice(null);
    try {
      const outcome = await openhumanTaskSourcesFetch(source.id);
      // Refresh the source list first (updates lastFetchAt/lastStatus);
      // `load()` resets the error/notice, so set the outcome message
      // *after* it so the message isn't immediately cleared.
      await load({ force: true });
      if (outcome.error) {
        setError(outcome.error);
      } else {
        setNotice(
          t('settings.taskSources.fetchResult')
            .replace('{routed}', String(outcome.routed))
            .replace('{fetched}', String(outcome.fetched))
            .replace('{pruned}', String(outcome.pruned ?? 0))
        );
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyKey(null);
    }
  };

  const syncAll = async () => {
    if (busyKey) return;
    setBusyKey('sync');
    setError(null);
    setNotice(null);
    try {
      const outcomes = await openhumanTaskSourcesSync();
      await load({ force: true });
      const firstError = outcomes.find(outcome => outcome.error)?.error;
      if (firstError) {
        setError(firstError);
      } else {
        const totals = formatSyncNotice(outcomes);
        setNotice(
          t('settings.taskSources.fetchResult')
            .replace('{routed}', String(totals.routed))
            .replace('{fetched}', String(totals.fetched))
            .replace('{pruned}', String(totals.pruned))
        );
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyKey(null);
    }
  };

  const removeSource = async (source: TaskSource) => {
    if (busyKey) return;
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

  const body = (
    <>
      <div className="space-y-1">
        <p className="text-xs text-content-muted">{t('settings.taskSources.description')}</p>
        <p className="text-xs text-content-faint">{t('settings.taskSources.connectHint')}</p>
      </div>

      {status && !status.enabled && (
        <div className="rounded-lg border border-amber-300 dark:border-amber-500/40 bg-amber-50 dark:bg-amber-500/10 px-4 py-3 text-sm text-amber-700 dark:text-amber-300">
          {t('settings.taskSources.disabledBanner')}
        </div>
      )}

      <SettingsStatusLine saving={false} savedNote={notice} error={error} savingLabel="" />

      {/* ── Add a source ─────────────────────────────────────────── */}
      <SettingsSection title={t('settings.taskSources.addTitle')}>
        <SettingsRow
          label={t('settings.taskSources.provider')}
          htmlFor="task-source-provider"
          stacked
          control={
            <SettingsSelect
              id="task-source-provider"
              value={provider}
              onChange={e => setProvider(e.target.value as TaskSourceProvider)}
              className="w-full">
              {PROVIDERS.map(p => (
                <option key={p} value={p}>
                  {providerLabel(p, t)}
                </option>
              ))}
            </SettingsSelect>
          }
        />

        <SettingsRow
          label={t('settings.taskSources.name')}
          htmlFor="task-source-name"
          stacked
          control={
            <SettingsTextField
              id="task-source-name"
              type="text"
              placeholder={t('settings.taskSources.namePlaceholder')}
              value={name}
              onChange={e => setName(e.target.value)}
              aria-label={t('settings.taskSources.name')}
            />
          }
        />

        <SettingsRow
          label={primaryLabel}
          htmlFor="task-source-primary"
          stacked
          control={
            <SettingsTextField
              id="task-source-primary"
              type="text"
              value={primary}
              onChange={e => setPrimary(e.target.value)}
              aria-label={primaryLabel}
            />
          }
        />

        {provider === 'notion' && (
          <SettingsRow
            stacked
            control={
              <div className="space-y-1">
                <Button
                  type="button"
                  variant="secondary"
                  size="xs"
                  disabled={busyKey !== null}
                  onClick={() => void browseDatabases()}>
                  {busyKey === 'databases'
                    ? t('settings.taskSources.notion.loadingDatabases')
                    : t('settings.taskSources.notion.browseDatabases')}
                </Button>
                {databases.length > 0 && (
                  <SettingsSelect
                    value={primary}
                    onChange={e => setPrimary(e.target.value)}
                    className="w-full mt-1">
                    <option value="">{t('settings.taskSources.notion.selectDatabase')}</option>
                    {databases.map(db => (
                      <option key={db.id} value={db.id}>
                        {db.title}
                      </option>
                    ))}
                  </SettingsSelect>
                )}
              </div>
            }
          />
        )}

        {provider === 'github' && (
          <SettingsRow
            label={t('settings.taskSources.github.labels')}
            htmlFor="task-source-labels"
            stacked
            control={
              <SettingsTextField
                id="task-source-labels"
                type="text"
                value={labels}
                onChange={e => setLabels(e.target.value)}
                aria-label={t('settings.taskSources.github.labels')}
              />
            }
          />
        )}

        <SettingsRow
          htmlFor="task-source-assigned"
          label={t('settings.taskSources.assignedToMe')}
          control={
            <SettingsCheckbox
              id="task-source-assigned"
              checked={assignedToMe}
              onCheckedChange={next => setAssignedToMe(next)}
              aria-label={t('settings.taskSources.assignedToMe')}
            />
          }
        />

        <div className="flex gap-2 px-4 py-3 border-t border-line-subtle">
          <Button
            type="button"
            variant="primary"
            size="xs"
            disabled={busyKey !== null}
            onClick={() => void addSource()}>
            {busyKey === 'add' ? t('settings.taskSources.adding') : t('settings.taskSources.add')}
          </Button>
          <Button
            type="button"
            variant="secondary"
            size="xs"
            disabled={busyKey !== null}
            onClick={() => void previewFilter()}>
            {t('settings.taskSources.preview')}
          </Button>
        </div>
      </SettingsSection>

      {/* ── Configured sources ───────────────────────────────────── */}
      <SettingsSection title={t('settings.taskSources.configured')}>
        <div className="px-4 py-3 border-b border-line-subtle">
          <Button
            type="button"
            variant="secondary"
            size="xs"
            disabled={loading || busyKey !== null || sources.length === 0}
            onClick={() => void syncAll()}>
            {busyKey === 'sync'
              ? t('settings.taskSources.syncing')
              : t('settings.taskSources.syncAll')}
          </Button>
        </div>

        {loading ? (
          <div className="px-4 py-3">
            <p className="text-sm text-content-faint">{t('common.loading')}</p>
          </div>
        ) : sources.length === 0 ? (
          <SettingsEmptyState label={t('settings.taskSources.empty')} />
        ) : (
          <ul className="divide-y divide-line-subtle dark:divide-neutral-800">
            {sources.map(source => (
              <li
                key={source.id}
                className="p-3 space-y-2"
                data-testid={`task-source-${source.id}`}>
                <div className="flex items-start justify-between gap-2">
                  <div>
                    <p className="text-sm font-medium text-content">
                      {source.name || providerLabel(source.provider, t)}
                    </p>
                    <p className="text-xs text-content-faint">
                      {providerLabel(source.provider, t)}
                      {source.target === 'agent_todo_proactive'
                        ? ` · ${t('settings.taskSources.proactive')}`
                        : ''}
                    </p>
                    <p className="text-xs text-content-faint">
                      {t('settings.taskSources.lastFetch')}:{' '}
                      {source.lastFetchAt
                        ? new Date(source.lastFetchAt).toLocaleString()
                        : t('settings.taskSources.never')}
                    </p>
                  </div>
                  <SettingsBadge variant={source.enabled ? 'success' : 'neutral'}>
                    {source.enabled
                      ? t('settings.taskSources.statusEnabled')
                      : t('settings.taskSources.statusDisabled')}
                  </SettingsBadge>
                </div>

                <div className="flex flex-wrap gap-2">
                  <Button
                    type="button"
                    variant="secondary"
                    size="xs"
                    disabled={busyKey !== null}
                    onClick={() => void toggleSource(source)}>
                    {source.enabled
                      ? t('settings.taskSources.disable')
                      : t('settings.taskSources.enable')}
                  </Button>
                  <Button
                    type="button"
                    variant="secondary"
                    size="xs"
                    disabled={busyKey !== null}
                    onClick={() => void fetchNow(source)}>
                    {busyKey === `fetch:${source.id}`
                      ? t('settings.taskSources.fetching')
                      : t('settings.taskSources.fetchNow')}
                  </Button>
                  <Button
                    type="button"
                    variant="secondary"
                    tone="danger"
                    size="xs"
                    disabled={busyKey !== null}
                    onClick={() => void removeSource(source)}>
                    {t('settings.taskSources.remove')}
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        )}

        <div className="px-4 py-3 border-t border-line-subtle">
          <Button
            type="button"
            variant="tertiary"
            size="xs"
            disabled={loading || busyKey !== null}
            onClick={() => void load()}>
            {t('settings.taskSources.refresh')}
          </Button>
        </div>
      </SettingsSection>
    </>
  );

  // Embedded inside the tabbed Integrations page: the parent owns the header,
  // so render just the padded body.
  if (embedded)
    return (
      <div className="p-4 pt-2 space-y-5" data-testid="task-sources-panel">
        {body}
      </div>
    );

  return <SettingsPanel testId="task-sources-panel">{body}</SettingsPanel>;
};

export default TaskSourcesPanel;
