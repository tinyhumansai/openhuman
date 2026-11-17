/**
 * One stored item read whole (`memory_items_get`): its full text, every
 * metadata field it carries, and its id, with a Forget action.
 *
 * debug logging: DEBUG=openhuman:memory:item
 */
import debug from 'debug';
import { useEffect, useId, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type Hit,
  memoryErrorMessage,
  memoryForget,
  memoryItemsGet,
  type MemoryMeta,
} from '../../services/api/memoryApi';
import { Badge, Button, ModalShell } from '../ui';
import { CenteredLoadingState } from '../ui/LoadingState';
import MemoryErrorAlert from './MemoryErrorAlert';
import { facetLabel } from './memoryFacetLabels';
import { formatTimestamp, KIND_VARIANT, kindLabel } from './memoryFormat';

const log = debug('openhuman:memory:item');

type Translate = (key: string, fallback?: string) => string;

interface MemoryItemDialogProps {
  id: string;
  onClose: () => void;
  /** Called after the item was forgotten. */
  onForgotten: (id: string) => void;
}

/** Every metadata field the item carries, labelled, in a fixed order. */
export function metaRows(meta: MemoryMeta, t: Translate): Array<[string, string]> {
  const rows: Array<[string, string | null | undefined]> = [
    [facetLabel('source', t), meta.source?.kind],
    [facetLabel('source_id', t), meta.source?.id],
    [facetLabel('workspace', t), meta.workspace],
    [facetLabel('folder', t), meta.folder],
    [facetLabel('file_path', t), meta.file_path],
    [facetLabel('language', t), meta.language],
    [facetLabel('repo', t), meta.repo],
    [t('memoryPage.explorer.meta.commit'), meta.commit],
    [facetLabel('url', t), meta.url],
    [facetLabel('thread', t), meta.thread_id],
    [
      t('memoryPage.explorer.meta.turns'),
      meta.turns ? `${meta.turns.first}–${meta.turns.last}` : null,
    ],
    [facetLabel('agent', t), meta.agent_id],
    [
      facetLabel('tool_call', t),
      meta.tool_call
        ? meta.tool_call.id
          ? `${meta.tool_call.name} (${meta.tool_call.id})`
          : meta.tool_call.name
        : null,
    ],
    [facetLabel('tag', t), meta.tags && meta.tags.length > 0 ? meta.tags.join(', ') : null],
    [t('memoryPage.explorer.meta.observedAt'), formatTimestamp(meta.observed_at)],
  ];
  return rows.filter((row): row is [string, string] => Boolean(row[1]));
}

export default function MemoryItemDialog({ id, onClose, onForgotten }: MemoryItemDialogProps) {
  const { t } = useT();
  const titleId = useId();
  const [item, setItem] = useState<Hit | null>(null);
  const [missing, setMissing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [forgetting, setForgetting] = useState(false);

  useEffect(() => {
    let cancelled = false;
    memoryItemsGet([id])
      .then(res => {
        if (cancelled) return;
        const found = res.items?.[0] ?? null;
        log('read id=%s found=%s', id, Boolean(found));
        setItem(found);
        setMissing(!found);
      })
      .catch(err => {
        if (cancelled) return;
        log('read failed: %o', err);
        setError(memoryErrorMessage(err, t));
      });
    return () => {
      cancelled = true;
    };
  }, [id, t]);

  const forget = async () => {
    setForgetting(true);
    setError(null);
    try {
      await memoryForget([id]);
      log('forgot id=%s', id);
      onForgotten(id);
      onClose();
    } catch (err) {
      log('forget failed: %o', err);
      setError(memoryErrorMessage(err, t));
    } finally {
      setForgetting(false);
    }
  };

  return (
    <ModalShell
      title={t('memoryPage.explorer.itemTitle')}
      titleId={titleId}
      onClose={onClose}
      maxWidthClassName="max-w-2xl"
      testId="memory-item-dialog"
      footer={
        <div className="flex justify-end gap-2">
          <Button
            type="button"
            variant="secondary"
            size="sm"
            data-testid="memory-item-close"
            onClick={onClose}>
            {t('common.close')}
          </Button>
          <Button
            type="button"
            variant="primary"
            tone="danger"
            size="sm"
            data-testid="memory-item-forget"
            disabled={!item || forgetting}
            onClick={() => void forget()}>
            {t('memoryPage.explorer.forget')}
          </Button>
        </div>
      }>
      <div className="space-y-4">
        {error !== null && <MemoryErrorAlert message={error} data-testid="memory-item-error" />}
        {missing ? (
          <p className="text-sm text-content-muted" data-testid="memory-item-missing">
            {t('memoryPage.explorer.itemMissing')}
          </p>
        ) : item === null ? (
          error === null && <CenteredLoadingState label={t('memoryPage.loading')} />
        ) : (
          <>
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant={KIND_VARIANT[item.kind] ?? 'neutral'}>
                {kindLabel(item.kind, t)}
              </Badge>
              <span className="truncate font-mono text-[11px] text-content-muted" title={item.id}>
                {item.id}
              </span>
            </div>
            <pre
              className="max-h-80 overflow-auto whitespace-pre-wrap break-words rounded-md bg-surface-muted p-3 text-sm text-content"
              data-testid="memory-item-text">
              {item.text}
            </pre>
            <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1 text-xs">
              {metaRows(item.meta, t).map(([label, value]) => (
                <div key={label} className="contents">
                  <dt className="font-medium text-content-secondary">{label}</dt>
                  <dd className="break-all font-mono text-content">{value}</dd>
                </div>
              ))}
            </dl>
          </>
        )}
      </div>
    </ModalShell>
  );
}
