/**
 * Memory → Learnings: the explicit facts, preferences and procedures memory
 * holds. Lists `memory_items_list` filtered to learnings (paged by cursor),
 * adds one with `memory_learn`, and deletes with `memory_forget`. Beliefs the
 * background builder distilled (tagged `belief`) carry a "Built belief" badge.
 *
 * Beliefs are built by a queued background job minutes behind the writes, so
 * an empty list while a build is pending reads as "still building", never as
 * "no memory" (#6718).
 *
 * debug logging: DEBUG=openhuman:memory:learnings
 */
import debug from 'debug';
import { type FormEvent, useCallback, useEffect, useState } from 'react';
import { LuTrash2 } from 'react-icons/lu';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type Hit,
  LEARNING_KINDS,
  type LearningKind,
  memoryErrorMessage,
  memoryForget,
  memoryItemsList,
  memoryJobsList,
  memoryLearn,
} from '../../services/api/memoryApi';
import { Button, Card, Label, NativeSelect, TextArea } from '../ui';
import { CenteredLoadingState } from '../ui/LoadingState';
import MemoryErrorAlert from './MemoryErrorAlert';
import MemoryHitRow from './MemoryHitRow';

const log = debug('openhuman:memory:learnings');

const PAGE_SIZE = 20;

export default function MemoryLearningsTab() {
  const { t } = useT();
  const [items, setItems] = useState<Hit[] | null>(null);
  const [cursor, setCursor] = useState<string | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [text, setText] = useState('');
  const [kind, setKind] = useState<LearningKind>('fact');
  const [adding, setAdding] = useState(false);
  const [deleting, setDeleting] = useState<string | null>(null);
  const [buildsPending, setBuildsPending] = useState(false);

  const kindLabel = (k: LearningKind): string => {
    switch (k) {
      case 'preference':
        return t('memoryPage.learnings.kindPreference');
      case 'fact':
        return t('memoryPage.learnings.kindFact');
      case 'procedure':
        return t('memoryPage.learnings.kindProcedure');
      case 'correction':
        return t('memoryPage.learnings.kindCorrection');
      default:
        return t('memoryPage.learnings.kindOther');
    }
  };

  const loadPage = useCallback(async (after: string | null) => {
    const page = await memoryItemsList({
      filter: { kinds: ['learning'] },
      limit: PAGE_SIZE,
      cursor: after ?? undefined,
    });
    log('page: %d item(s) more=%s', page.items?.length ?? 0, Boolean(page.next_cursor));
    return page;
  }, []);

  // Only asked when the list is empty; a failure leaves the plain empty state.
  const empty = items !== null && items.length === 0;
  useEffect(() => {
    if (!empty) return;
    let cancelled = false;
    memoryJobsList()
      .then(jobs => {
        if (cancelled) return;
        const pending = (jobs.pending ?? []).filter(job => job.job?.job === 'build_beliefs');
        log('empty list: %d belief build(s) pending', pending.length);
        setBuildsPending(pending.length > 0);
      })
      .catch(err => {
        if (cancelled) return;
        log('jobs list failed: %o', err);
        setBuildsPending(false);
      });
    return () => {
      cancelled = true;
    };
  }, [empty]);

  const reload = useCallback(async () => {
    setError(null);
    try {
      const page = await loadPage(null);
      setItems(page.items ?? []);
      setCursor(page.next_cursor ?? null);
    } catch (err) {
      log('list failed: %o', err);
      setError(memoryErrorMessage(err, t));
      setItems(prev => prev ?? []);
    }
  }, [loadPage, t]);

  useEffect(() => {
    let cancelled = false;
    loadPage(null)
      .then(page => {
        if (cancelled) return;
        setItems(page.items ?? []);
        setCursor(page.next_cursor ?? null);
      })
      .catch(err => {
        if (cancelled) return;
        log('list failed: %o', err);
        setError(memoryErrorMessage(err, t));
        setItems([]);
      });
    return () => {
      cancelled = true;
    };
  }, [loadPage, t]);

  const loadMore = async () => {
    if (!cursor) return;
    setLoadingMore(true);
    try {
      const page = await loadPage(cursor);
      setItems(prev => [...(prev ?? []), ...(page.items ?? [])]);
      setCursor(page.next_cursor ?? null);
    } catch (err) {
      setError(memoryErrorMessage(err, t));
    } finally {
      setLoadingMore(false);
    }
  };

  const add = async (event: FormEvent) => {
    event.preventDefault();
    const value = text.trim();
    if (!value || adding) return;
    setAdding(true);
    setError(null);
    try {
      const res = await memoryLearn({ text: value, kind });
      log('learned id=%s kind=%s', res.id, kind);
      setText('');
      await reload();
    } catch (err) {
      log('learn failed: %o', err);
      setError(memoryErrorMessage(err, t));
    } finally {
      setAdding(false);
    }
  };

  const remove = async (id: string) => {
    setDeleting(id);
    setError(null);
    try {
      await memoryForget([id]);
      setItems(prev => (prev ?? []).filter(item => item.id !== id));
    } catch (err) {
      log('forget failed: %o', err);
      setError(memoryErrorMessage(err, t));
    } finally {
      setDeleting(null);
    }
  };

  return (
    <div className="space-y-4 animate-fade-up" data-testid="memory-learnings-tab">
      <Card
        title={t('memoryPage.learnings.addTitle')}
        description={t('memoryPage.learnings.addDescription')}
        padded
        divided={false}>
        <form className="flex flex-col gap-3" onSubmit={e => void add(e)}>
          <Label htmlFor="memory-learning-text" className="sr-only">
            {t('memoryPage.learnings.textLabel')}
          </Label>
          <TextArea
            id="memory-learning-text"
            data-testid="memory-learning-input"
            rows={2}
            value={text}
            placeholder={t('memoryPage.learnings.placeholder')}
            onChange={e => setText(e.target.value)}
          />
          <div className="flex flex-wrap items-center justify-between gap-3">
            <NativeSelect
              aria-label={t('memoryPage.learnings.kindLabel')}
              data-testid="memory-learning-kind"
              value={kind}
              onChange={e => setKind(e.target.value as LearningKind)}>
              {LEARNING_KINDS.map(k => (
                <option key={k} value={k}>
                  {kindLabel(k)}
                </option>
              ))}
            </NativeSelect>
            <Button
              type="submit"
              variant="primary"
              size="sm"
              data-testid="memory-learning-add"
              disabled={adding || text.trim().length === 0}>
              {t('memoryPage.learnings.add')}
            </Button>
          </div>
        </form>
      </Card>

      {error !== null && <MemoryErrorAlert message={error} data-testid="memory-learnings-error" />}

      {items === null ? (
        <CenteredLoadingState label={t('memoryPage.loading')} />
      ) : (
        <Card title={t('memoryPage.learnings.listTitle')} data-testid="memory-learnings-list">
          {items.length === 0 && buildsPending ? (
            <p
              className="px-4 py-3 text-sm text-content-muted"
              data-testid="memory-learnings-deriving">
              {t('memoryPage.learnings.deriving')}
            </p>
          ) : items.length === 0 ? (
            <p
              className="px-4 py-3 text-sm text-content-muted"
              data-testid="memory-learnings-empty">
              {t('memoryPage.learnings.empty')}
            </p>
          ) : (
            <ul className="divide-y divide-line-subtle">
              {items.map(item => (
                <MemoryHitRow
                  key={item.id}
                  id={item.id}
                  kind={item.kind}
                  text={item.text}
                  meta={item.meta}
                  data-testid={`memory-learning-${item.id}`}
                  action={
                    <Button
                      type="button"
                      variant="tertiary"
                      size="xs"
                      iconOnly
                      aria-label={t('memoryPage.learnings.delete')}
                      title={t('memoryPage.learnings.delete')}
                      data-testid={`memory-learning-delete-${item.id}`}
                      disabled={deleting === item.id}
                      onClick={() => void remove(item.id)}>
                      <LuTrash2 className="h-3.5 w-3.5" aria-hidden />
                    </Button>
                  }
                />
              ))}
            </ul>
          )}
          {cursor && (
            <div className="px-4 py-3">
              <Button
                type="button"
                variant="secondary"
                size="sm"
                data-testid="memory-learnings-more"
                disabled={loadingMore}
                onClick={() => void loadMore()}>
                {t('memoryPage.loadMore')}
              </Button>
            </div>
          )}
        </Card>
      )}
    </div>
  );
}
