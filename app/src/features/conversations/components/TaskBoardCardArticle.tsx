import createDebug from 'debug';
import {
  LuArrowLeft,
  LuArrowRight,
  LuBot,
  LuCircleCheck,
  LuClipboardList,
  LuDatabase,
  LuExternalLink,
  LuPlay,
  LuShieldCheck,
  LuWrench,
} from 'react-icons/lu';

import Badge from '../../../components/ui/Badge';
import Button from '../../../components/ui/Button';
import { useT } from '../../../lib/i18n/I18nContext';
import type { TaskBoardCard, TaskBoardCardStatus } from '../../../types/turnState';
import { readSourceMetadata, sourceBadgeLabel } from './taskBoardMetadata';

const log = createDebug('app:conversations:task-board-card');

/** Whether a status owns a kanban column in the default 5-column set. */
export type IsColumnStatus = (status: TaskBoardCardStatus) => boolean;

/**
 * One card on the task board.
 *
 * Extracted out of `TaskKanbanBoard.tsx` when that file was split. Two things
 * changed with the move, both corrections rather than redesigns:
 *
 * - **The `ocean-*` classes were dead.** `ocean` is not defined anywhere in
 *   `tailwind.config.js`, so `bg-ocean-600` / `text-ocean-700` emitted no CSS
 *   at all and the "Approve", "Work task" and "View work" affordances rendered
 *   with no background. They are the shared {@link Button} / {@link Badge}
 *   primitives now, on the real `primary-*` tokens.
 * - **The hand-rolled metadata pills are {@link Badge}es**, which publish their
 *   tone as `data-variant` instead of a Tailwind class string. `sky-*` was a
 *   raw Tailwind palette scale that does not follow a user's theme; those
 *   source/evidence pills read as `primary` now.
 */
export function TaskBoardCardArticle({
  card,
  columnStatus,
  disabled,
  onMove,
  hasBriefActions,
  isColumnStatus,
  columnFor,
  onDecidePlan,
  onWorkTask,
  onViewSession,
  working,
  mutating,
  onOpenBrief,
}: {
  card: TaskBoardCard;
  columnStatus: TaskBoardCardStatus;
  disabled: boolean;
  onMove?: (card: TaskBoardCard, direction: -1 | 1) => void;
  hasBriefActions: boolean;
  isColumnStatus: IsColumnStatus;
  columnFor: (status: TaskBoardCardStatus) => TaskBoardCardStatus;
  onDecidePlan?: (card: TaskBoardCard, approve: boolean) => void;
  onWorkTask?: (card: TaskBoardCard) => void;
  onViewSession?: (card: TaskBoardCard) => void;
  working: boolean;
  mutating: boolean;
  onOpenBrief: () => void;
}) {
  const { t } = useT();
  const source = readSourceMetadata(card.sourceMetadata);
  const isDraggable = !disabled && Boolean(onMove);

  const handleDragStart = (e: React.DragEvent<HTMLElement>) => {
    log('drag start card=%s from column=%s', card.id, columnStatus);
    e.dataTransfer.setData('application/x-task-card-id', card.id);
    e.dataTransfer.effectAllowed = 'move';
  };

  return (
    <article
      draggable={isDraggable}
      onDragStart={isDraggable ? handleDragStart : undefined}
      className={`rounded-lg border border-line bg-surface px-2.5 py-2 shadow-xs transition-opacity ${
        mutating ? 'opacity-50' : 'opacity-100'
      } ${isDraggable ? 'cursor-grab active:cursor-grabbing' : ''}`}>
      <div className="flex items-start gap-2">
        <p className="min-w-0 flex-1 wrap-break-word text-xs font-medium leading-snug text-content">
          {card.title}
        </p>
        {card.sessionThreadId && onViewSession ? (
          <Button
            variant="tertiary"
            size="xs"
            title={t('conversations.taskKanban.viewWork')}
            onClick={() => onViewSession(card)}
            className="shrink-0 gap-1 px-1.5 text-[10px] text-primary-700 dark:text-primary-200">
            <LuExternalLink className="h-3 w-3 flex-none" />
            {t('conversations.taskKanban.viewWork')}
          </Button>
        ) : card.status === 'awaiting_approval' && onDecidePlan ? (
          <div className="flex shrink-0 items-center gap-1">
            <Button
              variant="primary"
              size="xs"
              title={t('chat.approval.approve')}
              disabled={disabled}
              onClick={() => onDecidePlan(card, true)}
              className="px-1.5 text-[10px]">
              {t('chat.approval.approve')}
            </Button>
            <Button
              variant="secondary"
              size="xs"
              title={t('chat.approval.deny')}
              disabled={disabled}
              onClick={() => onDecidePlan(card, false)}
              className="px-1.5 text-[10px]">
              {t('chat.approval.deny')}
            </Button>
          </div>
        ) : onWorkTask && card.status !== 'done' ? (
          <Button
            variant="primary"
            size="xs"
            title={t('conversations.taskKanban.workTask')}
            disabled={disabled || working}
            onClick={() => onWorkTask(card)}
            className="shrink-0 gap-1 px-1.5 text-[10px]">
            <LuPlay className="h-3 w-3" />
            {working
              ? t('conversations.taskKanban.startingTask')
              : t('conversations.taskKanban.workTask')}
          </Button>
        ) : onMove && isColumnStatus(columnFor(card.status)) ? (
          <div className="flex shrink-0 items-center gap-0.5">
            <Button
              iconOnly
              variant="tertiary"
              size="sm"
              title={t('conversations.taskKanban.moveLeft')}
              aria-label={t('conversations.taskKanban.moveLeft')}
              disabled={disabled || columnStatus === 'todo'}
              onClick={() => onMove(card, -1)}
              className="h-7 w-7 text-content-faint hover:text-content-secondary">
              <LuArrowLeft className="h-4 w-4" />
            </Button>
            <Button
              iconOnly
              variant="tertiary"
              size="sm"
              title={t('conversations.taskKanban.moveRight')}
              aria-label={t('conversations.taskKanban.moveRight')}
              disabled={disabled || columnStatus === 'done'}
              onClick={() => onMove(card, 1)}
              className="h-7 w-7 text-content-faint hover:text-content-secondary">
              <LuArrowRight className="h-4 w-4" />
            </Button>
          </div>
        ) : null}
      </div>
      <div className="mt-2 flex flex-wrap gap-1.5">
        {card.assignedAgent && (
          <Badge variant="primary" className="max-w-full gap-1 text-[10px]">
            <LuBot className="h-3 w-3 flex-none" />
            <span className="truncate">{card.assignedAgent}</span>
          </Badge>
        )}
        {card.allowedTools && card.allowedTools.length > 0 && (
          <Badge className="gap-1 text-[10px]">
            <LuWrench className="h-3 w-3" />
            {card.allowedTools.length}
          </Badge>
        )}
        {source && (
          <Badge variant="primary" className="max-w-full gap-1 text-[10px]">
            <LuDatabase className="h-3 w-3 flex-none" />
            <span className="truncate">{sourceBadgeLabel(source, t)}</span>
          </Badge>
        )}
        {source?.url && (
          <a
            href={source.url}
            target="_blank"
            rel="noreferrer"
            title={t('conversations.taskKanban.source.openExternal')}
            className="inline-flex items-center gap-1 rounded-md bg-surface-subtle px-1.5 py-0.5 text-[10px] text-content-secondary hover:bg-surface-strong">
            <LuExternalLink className="h-3 w-3" />
            {t('conversations.taskKanban.source.openExternalShort')}
          </a>
        )}
        {card.approvalMode && (
          <Badge variant="warning" className="gap-1 text-[10px]">
            <LuShieldCheck className="h-3 w-3" />
            {card.approvalMode === 'required'
              ? t('conversations.taskKanban.approval.requiredBadge')
              : t('conversations.taskKanban.approval.notRequiredBadge')}
          </Badge>
        )}
        {card.acceptanceCriteria && card.acceptanceCriteria.length > 0 && (
          <Badge variant="success" className="gap-1 text-[10px]">
            <LuCircleCheck className="h-3 w-3" />
            {card.acceptanceCriteria.length}
          </Badge>
        )}
        {/* Status badge: ready → "Ready to start" (sage); rejected → "Rejected" (coral) */}
        {card.status === 'ready' && (
          <Badge variant="success" className="text-[10px]">
            {t('conversations.taskKanban.statusBadge.ready')}
          </Badge>
        )}
        {card.status === 'rejected' && (
          <Badge variant="danger" className="text-[10px]">
            {t('conversations.taskKanban.statusBadge.rejected')}
          </Badge>
        )}
        {/* Evidence badge: shown on the card when evidence is present */}
        {card.evidence && card.evidence.length > 0 && (
          <Badge variant="primary" className="text-[10px]">
            {t('conversations.taskKanban.evidenceBadge').replace(
              '{count}',
              String(card.evidence.length)
            )}
          </Badge>
        )}
      </div>
      {card.objective && (
        <p className="mt-1 wrap-break-word text-[11px] leading-snug text-content-muted">
          {card.objective}
        </p>
      )}
      {card.notes && (
        <p className="mt-1 wrap-break-word text-[11px] leading-snug text-content-muted">
          {card.notes}
        </p>
      )}
      {/* Blocker text: always shown for blocked cards (column or status) */}
      {card.blocker && (card.status === 'blocked' || columnStatus === 'blocked') && (
        <p className="mt-1 wrap-break-word text-[11px] leading-snug text-coral-600">
          {card.blocker}
        </p>
      )}
      {(hasBriefActions ||
        card.plan?.length ||
        card.allowedTools?.length ||
        card.acceptanceCriteria?.length ||
        card.evidence?.length ||
        card.objective ||
        card.assignedAgent ||
        card.approvalMode ||
        source) && (
        <Button
          variant="tertiary"
          size="xs"
          onClick={onOpenBrief}
          className="mt-2 gap-1 px-0 text-[11px] text-primary-600 hover:bg-transparent hover:underline dark:text-primary-300">
          <LuClipboardList className="h-3 w-3" />
          {t('conversations.taskKanban.briefButton')}
        </Button>
      )}
    </article>
  );
}
