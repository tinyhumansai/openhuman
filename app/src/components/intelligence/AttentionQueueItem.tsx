/**
 * AttentionQueueItem — one row in the "Needs you" zone: a kind chip, the
 * instance title, a one-line detail (or unread count), and a single action
 * button whose verb depends on the kind (Review an approval / Open a run or
 * chat).
 *
 * Presentational only. The parent supplies the {@link AttentionItem} and an
 * `onAction` callback that receives the item's {@link AttentionAction} — the
 * tab (layout-rewrite PR) routes it to the right surface.
 */
import type { ReactElement } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type {
  AttentionAction,
  AttentionItem,
  AttentionKind,
} from '../../lib/orchestration/orchestrationClient';

export interface AttentionQueueItemProps {
  item: AttentionItem;
  onAction?: (action: AttentionAction) => void;
}

const KIND_LABEL_KEY: Record<AttentionKind, string> = {
  approval: 'tinyplaceOrchestration.attention.kind.approval',
  'needs-input': 'tinyplaceOrchestration.attention.kind.needsInput',
  unread: 'tinyplaceOrchestration.attention.kind.unread',
};

/** Left accent + chip tone per kind. Approvals and blocked runs read amber
 * (action required); unread reads ocean (informational). */
const KIND_TONE: Record<AttentionKind, { accent: string; chip: string }> = {
  approval: {
    accent: 'border-l-amber-500',
    chip: 'bg-amber-100 text-amber-700 dark:bg-amber-500/15 dark:text-amber-300',
  },
  'needs-input': {
    accent: 'border-l-amber-500',
    chip: 'bg-amber-100 text-amber-700 dark:bg-amber-500/15 dark:text-amber-300',
  },
  unread: {
    accent: 'border-l-ocean-500',
    chip: 'bg-ocean-100 text-ocean-700 dark:bg-ocean-500/15 dark:text-ocean-300',
  },
};

export default function AttentionQueueItem({
  item,
  onAction,
}: AttentionQueueItemProps): ReactElement {
  const { t } = useT();
  const tone = KIND_TONE[item.kind];
  // Approvals are decided ("Review"); everything else is navigated to ("Open").
  const actionLabel =
    item.kind === 'approval'
      ? t('tinyplaceOrchestration.attention.review')
      : t('tinyplaceOrchestration.attention.open');
  // Unread ships a localized label + a count pill (no summary string); the
  // other kinds carry a data summary from their source domain.
  const detail =
    item.kind === 'unread' ? t('tinyplaceOrchestration.attention.unread') : item.summary;

  return (
    <div
      data-testid={`attention-item-${item.id}`}
      data-kind={item.kind}
      className={`flex items-center gap-3 border-l-2 ${tone.accent} bg-surface px-3 py-2`}>
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5">
          <span
            className={`flex-none rounded-full px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${tone.chip}`}>
            {t(KIND_LABEL_KEY[item.kind])}
          </span>
          <span className="truncate text-xs font-semibold text-content">{item.title}</span>
        </span>
        {detail ? (
          <span className="mt-0.5 block truncate text-[11px] text-content-muted">{detail}</span>
        ) : null}
      </span>
      {item.kind === 'unread' && item.count !== undefined ? (
        <span
          data-testid="attention-item-count"
          className="flex-none rounded-full bg-ocean-500 px-1.5 py-0.5 text-[10px] font-semibold text-content-inverted">
          {item.count}
        </span>
      ) : null}
      <button
        type="button"
        data-testid="attention-item-action"
        onClick={() => onAction?.(item.action)}
        className="flex-none rounded-md border border-line px-2 py-1 text-[11px] font-medium text-content-secondary transition hover:bg-surface-hover">
        {actionLabel}
      </button>
    </div>
  );
}
