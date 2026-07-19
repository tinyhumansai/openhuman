/**
 * AttentionQueue — the "Needs you" zone of the Orchestration tab: a priority-
 * ordered list of everything blocked on the user (pending approvals, agent runs
 * awaiting input, instances with unread messages).
 *
 * Presentational only. The parent (layout-rewrite PR) loads the queue via
 * `orchestrationClient.attention()` and passes an `onAction` router. An empty
 * queue renders a calm "all caught up" state rather than hiding — the zone is a
 * stable anchor the user learns to glance at.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { AttentionAction, AttentionQueue } from '../../lib/orchestration/orchestrationClient';
import AttentionQueueItem from './AttentionQueueItem';

export interface AttentionQueueProps {
  queue: AttentionQueue | null;
  loading?: boolean;
  onAction?: (action: AttentionAction) => void;
}

export default function AttentionQueueView({
  queue,
  loading,
  onAction,
}: AttentionQueueProps): React.ReactElement | null {
  const { t } = useT();

  // Nothing to show until the first load resolves — the parent renders its own
  // shell; we stay out of the way rather than flashing an empty state.
  if (loading && !queue) return null;

  const total = queue?.counts.total ?? 0;

  return (
    <section data-testid="attention-queue" className="flex flex-col">
      <header className="flex items-center justify-between px-3 py-2">
        <h3 className="text-[11px] font-semibold uppercase tracking-wide text-content-secondary">
          {t('tinyplaceOrchestration.attention.title')}
        </h3>
        {total > 0 ? (
          <span
            data-testid="attention-queue-count"
            className="flex-none rounded-full bg-amber-500 px-1.5 py-0.5 text-[10px] font-semibold text-content-inverted">
            {total}
          </span>
        ) : null}
      </header>
      {total === 0 ? (
        <p data-testid="attention-queue-empty" className="px-3 py-2 text-[11px] text-content-faint">
          {t('tinyplaceOrchestration.attention.empty')}
        </p>
      ) : (
        <div className="flex flex-col gap-1">
          {queue?.items.map(item => (
            <AttentionQueueItem key={item.id} item={item} onAction={onAction} />
          ))}
        </div>
      )}
    </section>
  );
}
