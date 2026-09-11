import createDebug from 'debug';
import { useMemo, useState } from 'react';
import { LuDatabase } from 'react-icons/lu';

import {
  CollapsibleContent,
  CollapsibleRoot,
  CollapsibleTrigger,
} from '../../../components/ui/Collapsible';
import { useT } from '../../../lib/i18n/I18nContext';
import type { TaskBoard, TaskBoardCard, TaskBoardCardStatus } from '../../../types/turnState';
import { TaskBoardCardArticle } from './TaskBoardCardArticle';
import {
  COLUMN_ACCENT,
  COLUMN_DEFS,
  type ColumnDef,
  columnFor,
  isColumnStatus,
} from './taskBoardColumns';
import { readSourceMetadata } from './taskBoardMetadata';
import { TaskBriefDialog } from './TaskBriefDialog';
import { TaskSourceControls } from './TaskSourceControls';

export type { ColumnDef } from './taskBoardColumns';

const log = createDebug('app:conversations:task-kanban');

const TASK_SOURCES_THREAD_ID = 'task-sources';

interface TaskKanbanBoardProps {
  board: TaskBoard;
  disabled?: boolean;
  /** Column layout. Defaults to the 5-column set; the orchestrator board passes
   *  a 4-column set (Pending / Active / Blocked / Completed). */
  columns?: ColumnDef[];
  headerTitleKey?: string;
  /** Hide the board's own "Tasks" title row — used where the caller already
   *  renders a heading for the board, to avoid a doubled-up title. */
  hideHeader?: boolean;
  onMove?: (card: TaskBoardCard, status: TaskBoardCardStatus) => void;
  onUpdateCard?: (card: TaskBoardCard, nextCard: TaskBoardCard) => void;
  onDeleteCard?: (card: TaskBoardCard) => void;
  /** Approve/reject a card awaiting plan approval. */
  onDecidePlan?: (card: TaskBoardCard, approve: boolean) => void;
  /** Start work on a card from a higher-level task board. */
  onWorkTask?: (card: TaskBoardCard) => void;
  /** Jump to the card's agent session in Conversations. Shown on any card that
   *  carries a `sessionThreadId` (a run is live or has happened). */
  onViewSession?: (card: TaskBoardCard) => void;
  workingCardId?: string | null;
  /** Card id currently being mutated (move/update) — shows a loading indicator. */
  mutatingCardId?: string | null;
}

/**
 * The task board: one column per status, cards draggable between them, and a
 * per-card brief that opens as a modal.
 *
 * The 1243-line original was split into `TaskBoardCardArticle`,
 * `TaskSourceControls`, `TaskBriefDialog`, `taskBoardColumns` and
 * `taskBoardMetadata`; what stays here is the board itself. Two behavioural
 * corrections rode along with the split — see those files — and the
 * sources strip is now the shared Radix {@link CollapsibleRoot}, so its
 * trigger carries real `aria-expanded`/`aria-controls` wiring rather than a
 * hand-set `aria-expanded` on a `<button>` with no `aria-controls` at all.
 */
export function TaskKanbanBoard({
  board,
  disabled = false,
  columns = COLUMN_DEFS,
  headerTitleKey = 'conversations.taskKanban.title',
  hideHeader = false,
  onMove,
  onUpdateCard,
  onDeleteCard,
  onDecidePlan,
  onWorkTask,
  onViewSession,
  workingCardId = null,
  mutatingCardId = null,
}: TaskKanbanBoardProps) {
  const { t } = useT();
  const [selectedCardId, setSelectedCardId] = useState<string | null>(null);
  const [sourceControlsOpen, setSourceControlsOpen] = useState(false);
  const [dragOverColumn, setDragOverColumn] = useState<TaskBoardCardStatus | null>(null);

  // Per-instance column layout so a caller can override it (e.g. the
  // orchestrator's 4-column board). The status→column mapping helpers stay
  // shared, since any custom `columns` is a subset of the default statuses.
  const columnDefs = columns;
  const statusIndex = useMemo(
    () => new Map(columnDefs.map((column, index) => [column.status, index])),
    [columnDefs]
  );

  const selectedCard = useMemo(
    () => board.cards.find(card => card.id === selectedCardId) ?? null,
    [board.cards, selectedCardId]
  );
  const isTaskSourcesBoard = board.threadId === TASK_SOURCES_THREAD_ID;
  const hasSourceCards = board.cards.some(card => readSourceMetadata(card.sourceMetadata));
  const showSourceControls = isTaskSourcesBoard || hasSourceCards;

  // Always render (even with 0 cards) so a live agent board stays visible.
  const cardsByStatus = columnDefs.reduce(
    (acc, column) => {
      acc[column.status] = [];
      return acc;
    },
    {} as Record<TaskBoardCardStatus, TaskBoardCard[]>
  );

  for (const card of [...board.cards].sort((a, b) => a.order - b.order)) {
    cardsByStatus[columnFor(card.status)]?.push(card);
  }

  const moveCard = (card: TaskBoardCard, direction: -1 | 1) => {
    const current = statusIndex.get(columnFor(card.status)) ?? 0;
    const next = columnDefs[current + direction]?.status;
    if (!next || disabled) return;
    log('move card=%s %s -> %s', card.id, card.status, next);
    onMove?.(card, next);
  };

  // Cards can only be moved when the board is enabled and a handler exists.
  // Gate every drag-and-drop entry point on this so a disabled board cannot be
  // mutated via drop events (parity with moveCard's `disabled` guard above).
  const canMoveCards = !disabled && Boolean(onMove);

  const handleDragOver = (e: React.DragEvent<HTMLElement>, columnStatus: TaskBoardCardStatus) => {
    if (!canMoveCards) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'move';
    setDragOverColumn(columnStatus);
  };

  const handleDragLeave = (e: React.DragEvent<HTMLElement>) => {
    if (!canMoveCards) return;
    if (!e.currentTarget.contains(e.relatedTarget as Node)) {
      setDragOverColumn(null);
    }
  };

  const handleDrop = (e: React.DragEvent<HTMLElement>, targetColumnStatus: TaskBoardCardStatus) => {
    if (!canMoveCards) return;
    e.preventDefault();
    setDragOverColumn(null);
    const cardId = e.dataTransfer.getData('application/x-task-card-id');
    if (!cardId) return;
    const draggedCard = board.cards.find(c => c.id === cardId);
    if (!draggedCard) return;
    const currentColumn = columnFor(draggedCard.status);
    if (currentColumn === targetColumnStatus) return;
    log('drop card=%s %s -> %s', cardId, currentColumn, targetColumnStatus);
    onMove?.(draggedCard, targetColumnStatus);
  };

  return (
    <div className="py-3">
      {!hideHeader && (
        // The header row and the sources strip share one disclosure: the strip
        // is the collapsible body, the "Sources" chip is its trigger.
        <CollapsibleRoot
          open={sourceControlsOpen}
          onOpenChange={next => {
            log('task sources controls open=%s', next);
            setSourceControlsOpen(next);
          }}>
          <div className="mb-2 flex items-center justify-between gap-3">
            <h4 className="text-xs font-semibold uppercase tracking-wide text-content-muted">
              {t(headerTitleKey)}
            </h4>
            <div className="flex items-center gap-2">
              {showSourceControls && (
                <CollapsibleTrigger
                  size="sm"
                  className="w-auto justify-start gap-1 rounded-md border border-line px-2 py-1 text-[10px] font-medium text-content-secondary">
                  <LuDatabase className="h-3 w-3" />
                  {t('conversations.taskKanban.sourcesButton')}
                </CollapsibleTrigger>
              )}
              <span className="text-[10px] text-content-faint">{board.cards.length}</span>
            </div>
          </div>
          {showSourceControls && (
            <CollapsibleContent size="sm" className="px-0 pb-0">
              <TaskSourceControls disabled={disabled} compact={!isTaskSourcesBoard} />
            </CollapsibleContent>
          )}
        </CollapsibleRoot>
      )}
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-3 lg:grid-cols-5">
        {columnDefs.map(column => {
          const cards = cardsByStatus[column.status];
          const isBlockedColumn = column.status === 'blocked';
          const isDragTarget = dragOverColumn === column.status;
          const accentClass = COLUMN_ACCENT[column.status] ?? '';
          return (
            <section
              key={column.status}
              className={`min-w-0 rounded-lg bg-surface-muted p-2 ${accentClass} ${
                isDragTarget ? 'bg-primary-50/60 ring-2 ring-primary-400 dark:bg-primary-500/5' : ''
              }`}
              onDragOver={canMoveCards ? e => handleDragOver(e, column.status) : undefined}
              onDragLeave={canMoveCards ? handleDragLeave : undefined}
              onDrop={canMoveCards ? e => handleDrop(e, column.status) : undefined}>
              <div className="mb-2 flex items-center justify-between gap-2">
                <h5 className="truncate text-[11px] font-medium text-content-secondary">
                  {t(column.labelKey)}
                </h5>
                <span className="text-[10px] text-content-faint">{cards.length}</span>
              </div>
              {/* "Needs your input" banner at top of Blocked column */}
              {isBlockedColumn && cards.length > 0 && (
                <div className="mb-2 rounded-md bg-coral-50 px-2 py-1.5 text-[10px] font-medium text-coral-700 dark:bg-coral-500/10 dark:text-coral-300">
                  {t('conversations.taskKanban.needsInput')}
                </div>
              )}
              <div className="space-y-2">
                {cards.length === 0 ? (
                  <p className="py-2 text-center text-[10px] text-content-faint">
                    {t('conversations.taskKanban.emptyColumn')}
                  </p>
                ) : (
                  cards.map(card => (
                    <TaskBoardCardArticle
                      key={card.id}
                      card={card}
                      columnStatus={column.status}
                      disabled={disabled}
                      onMove={onMove ? moveCard : undefined}
                      hasBriefActions={Boolean(onUpdateCard || onDeleteCard)}
                      isColumnStatus={isColumnStatus}
                      columnFor={columnFor}
                      onDecidePlan={onDecidePlan}
                      onWorkTask={onWorkTask}
                      onViewSession={onViewSession}
                      working={workingCardId === card.id}
                      mutating={mutatingCardId === card.id}
                      onOpenBrief={() => setSelectedCardId(card.id)}
                    />
                  ))
                )}
              </div>
            </section>
          );
        })}
      </div>
      {selectedCard && (
        <TaskBriefDialog
          card={selectedCard}
          disabled={disabled}
          onClose={() => setSelectedCardId(null)}
          onUpdate={onUpdateCard}
          onDelete={onDeleteCard}
        />
      )}
    </div>
  );
}
