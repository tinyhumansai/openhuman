/**
 * Memory → Brain: the documents every agent shares, filed by source type
 * (pdf, markdown, notion, github, web, …).
 *
 * - Per-source document counts (`memory_brain_sources`), each with a "Forget
 *   source" action behind a confirm dialog (`memory_brain_forget`).
 * - Search across the brain ({@link MemoryBrainSearch}).
 * - "Add document" from pasted text or a file path (`memory_brain_ingest`).
 * - Below, the synced sources that keep feeding it ({@link MemorySyncedSources}).
 *
 * debug logging: DEBUG=openhuman:memory:brain
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';
import { LuPlus } from 'react-icons/lu';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type BrainIngestRequest,
  type BrainSources,
  memoryBrainForget,
  memoryBrainIngest,
  memoryBrainSources,
  memoryErrorMessage,
} from '../../services/api/memoryApi';
import { Alert, AlertDescription, Button, Card, ConfirmDialog } from '../ui';
import { CenteredLoadingState } from '../ui/LoadingState';
import MemoryBrainIngestDialog from './MemoryBrainIngestDialog';
import MemoryBrainSearch from './MemoryBrainSearch';
import MemoryErrorAlert from './MemoryErrorAlert';
import { fill } from './memoryFormat';
import { brainSourceLabel } from './memoryLifecycleLabels';
import MemorySyncedSources from './MemorySyncedSources';

const log = debug('openhuman:memory:brain');

export default function MemoryBrainTab() {
  const { t } = useT();
  const [brain, setBrain] = useState<BrainSources | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [addError, setAddError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [forgetTarget, setForgetTarget] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      const res = await memoryBrainSources();
      log('sources: %d unfiled=%d', res.sources?.length ?? 0, res.unfiled ?? 0);
      setBrain({ ...res, sources: res.sources ?? [] });
      setError(null);
    } catch (err) {
      log('brain_sources failed: %o', err);
      setError(memoryErrorMessage(err, t));
      setBrain(prev => prev ?? { root: '', sources: [], unfiled: 0 });
    }
  }, [t]);

  useEffect(() => {
    let cancelled = false;
    memoryBrainSources()
      .then(res => {
        if (cancelled) return;
        log('sources: %d unfiled=%d', res.sources?.length ?? 0, res.unfiled ?? 0);
        setBrain({ ...res, sources: res.sources ?? [] });
      })
      .catch(err => {
        if (cancelled) return;
        log('brain_sources failed: %o', err);
        setError(memoryErrorMessage(err, t));
        setBrain({ root: '', sources: [], unfiled: 0 });
      });
    return () => {
      cancelled = true;
    };
  }, [t]);

  const ingest = async (req: BrainIngestRequest): Promise<boolean> => {
    setSaving(true);
    setAddError(null);
    setNotice(null);
    try {
      const res = await memoryBrainIngest(req);
      log('ingested id=%s source=%s replayed=%s', res.id, res.source, res.replayed);
      setNotice(
        res.replayed
          ? t('memoryPage.brain.ingestReplayed')
          : fill(t('memoryPage.brain.ingestDone'), { source: brainSourceLabel(res.source, t) })
      );
      await reload();
      return true;
    } catch (err) {
      log('brain_ingest failed: %o', err);
      setAddError(memoryErrorMessage(err, t));
      return false;
    } finally {
      setSaving(false);
    }
  };

  const confirmForget = async () => {
    if (!forgetTarget) return;
    const source = forgetTarget;
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      const res = await memoryBrainForget(source);
      log('forgot source=%s n=%d', source, res.forgotten);
      setNotice(
        fill(t('memoryPage.brain.forgotten'), {
          count: res.forgotten,
          source: brainSourceLabel(source, t),
        })
      );
      setForgetTarget(null);
      await reload();
    } catch (err) {
      log('brain_forget failed: %o', err);
      setError(memoryErrorMessage(err, t));
      setForgetTarget(null);
    } finally {
      setSaving(false);
    }
  };

  if (brain === null) return <CenteredLoadingState label={t('memoryPage.loading')} />;

  return (
    <div className="space-y-4 animate-fade-up" data-testid="memory-brain-tab">
      {error !== null && <MemoryErrorAlert message={error} data-testid="memory-brain-error" />}
      {notice !== null && (
        <Alert variant="success" data-testid="memory-brain-notice">
          <AlertDescription>{notice}</AlertDescription>
        </Alert>
      )}

      <Card
        title={t('memoryPage.brain.sourcesTitle')}
        description={t('memoryPage.brain.sourcesDescription')}
        headerRight={
          <Button
            type="button"
            variant="primary"
            size="sm"
            analyticsId="memory-brain-add-document"
            data-testid="memory-brain-add"
            onClick={() => {
              setAddError(null);
              setAdding(true);
            }}>
            <LuPlus className="h-3.5 w-3.5" aria-hidden />
            {t('memoryPage.brain.addDocument')}
          </Button>
        }
        data-testid="memory-brain-sources">
        {brain.sources.length === 0 && !brain.unfiled ? (
          <p className="px-4 py-3 text-sm text-content-muted" data-testid="memory-brain-empty">
            {t('memoryPage.brain.empty')}
          </p>
        ) : (
          <ul className="divide-y divide-line-subtle">
            {brain.sources.map(entry => (
              <li
                key={entry.source}
                className="flex items-center gap-3 px-4 py-2.5"
                data-testid={`memory-brain-source-${entry.source}`}>
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-semibold text-content">
                    {brainSourceLabel(entry.source, t)}
                  </p>
                  <p className="text-xs text-content-muted">
                    {fill(t('memoryPage.brain.documents'), { count: entry.documents })}
                  </p>
                </div>
                <Button
                  type="button"
                  variant="tertiary"
                  tone="danger"
                  size="xs"
                  analyticsId="memory-brain-forget-source"
                  data-testid={`memory-brain-source-${entry.source}-forget`}
                  onClick={() => setForgetTarget(entry.source)}>
                  {t('memoryPage.brain.forgetSource')}
                </Button>
              </li>
            ))}
            {brain.unfiled > 0 && (
              <li
                className="px-4 py-2.5 text-xs text-content-muted"
                data-testid="memory-brain-unfiled">
                {fill(t('memoryPage.brain.unfiled'), { count: brain.unfiled })}
              </li>
            )}
          </ul>
        )}
      </Card>

      <MemoryBrainSearch sources={brain.sources.map(entry => entry.source)} />

      <MemorySyncedSources />

      {adding && (
        <MemoryBrainIngestDialog
          saving={saving}
          error={addError}
          onSubmit={ingest}
          onClose={() => setAdding(false)}
        />
      )}

      {forgetTarget !== null && (
        <ConfirmDialog
          title={t('memoryPage.brain.forgetTitle')}
          testId="memory-brain-forget"
          confirmTestId="memory-brain-forget-confirm"
          destructive
          busy={saving}
          confirmLabel={t('memoryPage.brain.forgetSource')}
          body={
            <p className="text-sm text-content-secondary">
              {fill(t('memoryPage.brain.forgetBody'), {
                source: brainSourceLabel(forgetTarget, t),
              })}
            </p>
          }
          onConfirm={() => void confirmForget()}
          onCancel={() => setForgetTarget(null)}
        />
      )}
    </div>
  );
}
