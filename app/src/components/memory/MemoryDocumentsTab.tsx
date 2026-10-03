/**
 * Memory → Documents: the registry of synced sources (folders, files, links,
 * GitHub repos, RSS feeds, Composio toolkits). Lists them with status, item
 * count, last sync and any error; adds, syncs one or all, and removes (with an
 * opt-in to forget the items the source stored).
 *
 * debug logging: DEBUG=openhuman:memory:documents
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';
import { LuPlus, LuRefreshCw } from 'react-icons/lu';

import { useT } from '../../lib/i18n/I18nContext';
import {
  memoryErrorMessage,
  memorySourcesAdd,
  memorySourcesList,
  memorySourcesRemove,
  memorySourcesSync,
  type Source,
  type SourceAddRequest,
} from '../../services/api/memoryApi';
import { Alert, AlertDescription, Badge, Button, Card, Checkbox, ConfirmDialog } from '../ui';
import { CenteredLoadingState } from '../ui/LoadingState';
import MemoryAddSourceDialog from './MemoryAddSourceDialog';
import { fill, formatTimestamp } from './memoryFormat';
import { SOURCE_STATUS_VARIANT, sourceKindLabel, sourceStatusLabel } from './memorySourceLabels';

const log = debug('openhuman:memory:documents');

const SYNC_POLL_MS = 5_000;

export default function MemoryDocumentsTab() {
  const { t } = useT();
  const [sources, setSources] = useState<Source[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [addError, setAddError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [syncing, setSyncing] = useState<Set<string>>(() => new Set());
  const [removeTarget, setRemoveTarget] = useState<Source | null>(null);
  const [forgetItems, setForgetItems] = useState(false);

  const reload = useCallback(async () => {
    try {
      const res = await memorySourcesList();
      log('sources: %d', res.sources?.length ?? 0);
      setSources(res.sources ?? []);
      setError(null);
    } catch (err) {
      log('list failed: %o', err);
      setError(memoryErrorMessage(err));
      setSources(prev => prev ?? []);
    }
  }, []);

  // Initial load. `reload` re-reads on demand (polling); the effect awaits
  // before touching state so the first render is never re-rendered in place.
  useEffect(() => {
    let cancelled = false;
    memorySourcesList()
      .then(res => {
        if (cancelled) return;
        log('sources: %d', res.sources?.length ?? 0);
        setSources(res.sources ?? []);
      })
      .catch(err => {
        if (cancelled) return;
        log('list failed: %o', err);
        setError(memoryErrorMessage(err));
        setSources([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // While anything is syncing, re-read the registry so status, item counts and
  // errors move on their own; idle registries are not polled.
  const anySyncing = (sources ?? []).some(s => s.status === 'syncing');
  useEffect(() => {
    if (!anySyncing) return;
    const timer = setInterval(() => void reload(), SYNC_POLL_MS);
    return () => clearInterval(timer);
  }, [anySyncing, reload]);

  const add = async (req: SourceAddRequest): Promise<boolean> => {
    setSaving(true);
    setAddError(null);
    try {
      const res = await memorySourcesAdd(req);
      log('added source id=%s kind=%s', res.source.id, res.source.kind);
      setSources(prev => [...(prev ?? []).filter(s => s.id !== res.source.id), res.source]);
      return true;
    } catch (err) {
      log('add failed: %o', err);
      setAddError(memoryErrorMessage(err));
      return false;
    } finally {
      setSaving(false);
    }
  };

  const sync = async (id?: string) => {
    const key = id ?? '*';
    setSyncing(prev => new Set(prev).add(key));
    setError(null);
    try {
      const res = await memorySourcesSync(id);
      log('sync started: %d source(s)', res.started?.length ?? 0);
      const started = new Set(res.started ?? []);
      setSources(prev =>
        (prev ?? []).map(s => (started.has(s.id) ? { ...s, status: 'syncing', error: null } : s))
      );
    } catch (err) {
      log('sync failed: %o', err);
      setError(memoryErrorMessage(err));
    } finally {
      setSyncing(prev => {
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
    }
  };

  const confirmRemove = async () => {
    if (!removeTarget) return;
    const target = removeTarget;
    setSaving(true);
    setError(null);
    try {
      await memorySourcesRemove(target.id, forgetItems);
      log('removed source id=%s forget=%s', target.id, forgetItems);
      setSources(prev => (prev ?? []).filter(s => s.id !== target.id));
      setRemoveTarget(null);
    } catch (err) {
      log('remove failed: %o', err);
      setError(memoryErrorMessage(err));
      setRemoveTarget(null);
    } finally {
      setSaving(false);
    }
  };

  if (sources === null) return <CenteredLoadingState label={t('memoryPage.loading')} />;

  return (
    <div className="space-y-4 animate-fade-up" data-testid="memory-documents-tab">
      {error !== null && (
        <Alert variant="destructive" data-testid="memory-documents-error">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}

      <Card
        title={t('memoryPage.documents.listTitle')}
        description={t('memoryPage.documents.listDescription')}
        headerRight={
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="secondary"
              size="sm"
              data-testid="memory-sources-sync-all"
              disabled={sources.length === 0 || syncing.has('*')}
              onClick={() => void sync()}>
              <LuRefreshCw className="h-3.5 w-3.5" aria-hidden />
              {t('memoryPage.documents.syncAll')}
            </Button>
            <Button
              type="button"
              variant="primary"
              size="sm"
              data-testid="memory-sources-add"
              onClick={() => {
                setAddError(null);
                setAdding(true);
              }}>
              <LuPlus className="h-3.5 w-3.5" aria-hidden />
              {t('memoryPage.documents.add')}
            </Button>
          </div>
        }
        data-testid="memory-sources">
        {sources.length === 0 ? (
          <p className="px-4 py-3 text-sm text-content-muted" data-testid="memory-sources-empty">
            {t('memoryPage.documents.empty')}
          </p>
        ) : (
          <ul className="divide-y divide-line-subtle">
            {sources.map(source => {
              const lastSync = formatTimestamp(source.last_sync_at);
              return (
                <li
                  key={source.id}
                  className="flex items-start gap-3 px-4 py-3"
                  data-testid={`memory-source-${source.id}`}>
                  <div className="min-w-0 flex-1 space-y-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="truncate text-sm font-semibold text-content">
                        {source.label || source.target}
                      </span>
                      <Badge variant="neutral">{sourceKindLabel(source.kind, t)}</Badge>
                      <Badge
                        variant={SOURCE_STATUS_VARIANT[source.status] ?? 'neutral'}
                        data-testid={`memory-source-${source.id}-status`}>
                        {sourceStatusLabel(source.status, t)}
                      </Badge>
                    </div>
                    <p className="truncate font-mono text-xs text-content-muted">{source.target}</p>
                    <p className="text-xs text-content-muted">
                      {fill(t('memoryPage.documents.items'), { count: source.items ?? 0 })}
                      {' · '}
                      {lastSync
                        ? fill(t('memoryPage.documents.lastSync'), { when: lastSync })
                        : t('memoryPage.documents.neverSynced')}
                      {source.schedule_mins
                        ? ` · ${fill(t('memoryPage.documents.every'), { mins: source.schedule_mins })}`
                        : ''}
                    </p>
                    {source.error && (
                      <p
                        className="text-xs text-coral-600 dark:text-coral-300"
                        data-testid={`memory-source-${source.id}-error`}>
                        {source.error}
                      </p>
                    )}
                  </div>
                  <div className="flex shrink-0 items-center gap-2">
                    <Button
                      type="button"
                      variant="secondary"
                      size="xs"
                      data-testid={`memory-source-${source.id}-sync`}
                      disabled={source.status === 'syncing' || syncing.has(source.id)}
                      onClick={() => void sync(source.id)}>
                      {t('memoryPage.documents.sync')}
                    </Button>
                    <Button
                      type="button"
                      variant="tertiary"
                      tone="danger"
                      size="xs"
                      data-testid={`memory-source-${source.id}-remove`}
                      onClick={() => {
                        setForgetItems(false);
                        setRemoveTarget(source);
                      }}>
                      {t('memoryPage.documents.remove')}
                    </Button>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </Card>

      {adding && (
        <MemoryAddSourceDialog
          saving={saving}
          error={addError}
          onSubmit={add}
          onClose={() => setAdding(false)}
        />
      )}

      {removeTarget && (
        <ConfirmDialog
          title={t('memoryPage.documents.removeTitle')}
          testId="memory-remove-source"
          confirmTestId="memory-remove-source-confirm"
          destructive
          busy={saving}
          confirmLabel={t('memoryPage.documents.remove')}
          body={
            <div className="space-y-3">
              <p className="text-sm text-content-secondary">
                {fill(t('memoryPage.documents.removeBody'), {
                  name: removeTarget.label || removeTarget.target,
                })}
              </p>
              <label className="flex items-center gap-2 text-sm text-content">
                <Checkbox
                  id="memory-remove-source-forget"
                  data-testid="memory-remove-source-forget"
                  checked={forgetItems}
                  onCheckedChange={setForgetItems}
                />
                {t('memoryPage.documents.forgetItems')}
              </label>
            </div>
          }
          onConfirm={() => void confirmRemove()}
          onCancel={() => setRemoveTarget(null)}
        />
      )}
    </div>
  );
}
