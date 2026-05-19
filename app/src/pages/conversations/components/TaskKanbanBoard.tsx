import { LuArrowLeft, LuArrowRight } from 'react-icons/lu';

import { useT } from '../../../lib/i18n/I18nContext';
import type { TaskBoard, TaskBoardCard, TaskBoardCardStatus } from '../../../types/turnState';

type ColumnDef = { status: TaskBoardCardStatus; labelKey: string };

const COLUMN_DEFS: ColumnDef[] = [
  { status: 'todo', labelKey: 'conversations.taskKanban.todo' },
  { status: 'in_progress', labelKey: 'conversations.taskKanban.inProgress' },
  { status: 'blocked', labelKey: 'conversations.taskKanban.blocked' },
  { status: 'done', labelKey: 'conversations.taskKanban.done' },
];

const STATUS_INDEX = new Map(COLUMN_DEFS.map((column, index) => [column.status, index]));

interface TaskKanbanBoardProps {
  board: TaskBoard;
  disabled?: boolean;
  onMove?: (card: TaskBoardCard, status: TaskBoardCardStatus) => void;
}

export function TaskKanbanBoard({ board, disabled = false, onMove }: TaskKanbanBoardProps) {
  const { t } = useT();

  if (board.cards.length === 0) return null;

  const cardsByStatus = COLUMN_DEFS.reduce(
    (acc, column) => {
      acc[column.status] = [];
      return acc;
    },
    {} as Record<TaskBoardCardStatus, TaskBoardCard[]>
  );

  for (const card of [...board.cards].sort((a, b) => a.order - b.order)) {
    cardsByStatus[card.status]?.push(card);
  }

  const moveCard = (card: TaskBoardCard, direction: -1 | 1) => {
    const current = STATUS_INDEX.get(card.status) ?? 0;
    const next = COLUMN_DEFS[current + direction]?.status;
    if (!next || disabled) return;
    onMove?.(card, next);
  };

  return (
    <div className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-3 shadow-sm">
      <div className="mb-2 flex items-center justify-between gap-3">
        <h4 className="text-xs font-semibold uppercase tracking-wide text-stone-500 dark:text-neutral-400">
          {t('conversations.taskKanban.title')}
        </h4>
        <span className="text-[10px] text-stone-400 dark:text-neutral-500">
          {board.cards.length}
        </span>
      </div>
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-4">
        {COLUMN_DEFS.map(column => (
          <section
            key={column.status}
            className="min-w-0 rounded-lg bg-stone-50 dark:bg-neutral-800/60 p-2">
            <div className="mb-2 flex items-center justify-between gap-2">
              <h5 className="truncate text-[11px] font-medium text-stone-600 dark:text-neutral-300">
                {t(column.labelKey)}
              </h5>
              <span className="text-[10px] text-stone-400 dark:text-neutral-500">
                {cardsByStatus[column.status].length}
              </span>
            </div>
            <div className="space-y-2">
              {cardsByStatus[column.status].map(card => (
                <article
                  key={card.id}
                  className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-2.5 py-2 shadow-sm">
                  <div className="flex items-start gap-2">
                    <p className="min-w-0 flex-1 break-words text-xs font-medium leading-snug text-stone-800 dark:text-neutral-100">
                      {card.title}
                    </p>
                    {onMove && (
                      <div className="flex flex-shrink-0 items-center gap-0.5">
                        <button
                          type="button"
                          title={t('conversations.taskKanban.moveLeft')}
                          aria-label={t('conversations.taskKanban.moveLeft')}
                          disabled={disabled || column.status === 'todo'}
                          onClick={() => moveCard(card, -1)}
                          className="flex h-5 w-5 items-center justify-center rounded-md text-stone-400 dark:text-neutral-500 transition-colors hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800 hover:text-stone-700 dark:hover:text-neutral-200 dark:text-neutral-200 disabled:opacity-25">
                          <LuArrowLeft className="h-3 w-3" />
                        </button>
                        <button
                          type="button"
                          title={t('conversations.taskKanban.moveRight')}
                          aria-label={t('conversations.taskKanban.moveRight')}
                          disabled={disabled || column.status === 'done'}
                          onClick={() => moveCard(card, 1)}
                          className="flex h-5 w-5 items-center justify-center rounded-md text-stone-400 dark:text-neutral-500 transition-colors hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800 hover:text-stone-700 dark:hover:text-neutral-200 dark:text-neutral-200 disabled:opacity-25">
                          <LuArrowRight className="h-3 w-3" />
                        </button>
                      </div>
                    )}
                  </div>
                  {card.notes && (
                    <p className="mt-1 break-words text-[11px] leading-snug text-stone-500 dark:text-neutral-400">
                      {card.notes}
                    </p>
                  )}
                  {card.status === 'blocked' && card.blocker && (
                    <p className="mt-1 break-words text-[11px] leading-snug text-coral-600">
                      {card.blocker}
                    </p>
                  )}
                </article>
              ))}
            </div>
          </section>
        ))}
      </div>
    </div>
  );
}
