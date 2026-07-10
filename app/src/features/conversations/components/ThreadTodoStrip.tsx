import React, { useMemo, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import type { TaskBoard, TaskBoardCard, TaskBoardCardStatus } from '../../../types/turnState';

/**
 * Compact, read-only strip that surfaces the *current conversation thread's*
 * task board ("todo list") directly above the composer. It mirrors hermes-agent's
 * pinned todo panel: a one-glance plan the agent maintains as it works through a
 * multi-step task.
 *
 * Read-only by design — the agent owns the board via its `todo` tool (mutations
 * arrive live through the `task_board_updated` socket event into
 * `taskBoardByThread`). Direct user editing lives in the full board UI, not here.
 *
 * This is NOT the Intelligence-tab kanban (that board is the global `user-tasks`
 * list). This strip is strictly scoped to the selected chat thread.
 */

/** Statuses that represent live, actionable work — shown in the strip. */
const ACTIVE_STATUSES: readonly TaskBoardCardStatus[] = [
  'in_progress',
  'todo',
  'ready',
  'awaiting_approval',
  'blocked',
];

/** Terminal "finished" status — counted toward progress, not listed. */
const DONE_STATUS: TaskBoardCardStatus = 'done';

/** Plain-text glyph per status (monochrome, terminal-style — like hermes). */
function statusGlyph(status: TaskBoardCardStatus): string {
  switch (status) {
    case 'in_progress':
      return '[~]';
    case 'blocked':
      return '[!]';
    case 'awaiting_approval':
      return '[?]';
    case 'done':
      return '[x]';
    case 'rejected':
      return '[-]';
    case 'todo':
    case 'ready':
    default:
      return '[ ]';
  }
}

/** Tailwind text-color token per status, matching the app's semantic palette. */
function statusColorClass(status: TaskBoardCardStatus): string {
  switch (status) {
    case 'in_progress':
      return 'text-primary-600 dark:text-primary-300 font-medium';
    case 'blocked':
      return 'text-coral dark:text-coral';
    case 'awaiting_approval':
      return 'text-amber-700 dark:text-amber-300';
    default:
      return 'text-content-secondary';
  }
}

interface Props {
  board: TaskBoard | null;
  /**
   * Decide a parked plan (`awaiting_approval` card). When provided, those cards —
   * and only those — gain inline Approve/Reject controls; every other card stays
   * read-only. Omit to keep the strip fully read-only.
   */
  onDecidePlan?: (card: TaskBoardCard, approve: boolean) => void;
  /**
   * Jump to a card's linked agent session. When provided, cards that carry a
   * `sessionThreadId` (stamped by the autonomous/manual task-session flow) gain a
   * "View work" affordance. Omit to hide it.
   */
  onViewSession?: (card: TaskBoardCard) => void;
  /** Disable the approve/reject controls (e.g. no thread selected). */
  disabled?: boolean;
}

export const ThreadTodoStrip: React.FC<Props> = ({
  board,
  onDecidePlan,
  onViewSession,
  disabled = false,
}) => {
  const { t } = useT();
  const [collapsed, setCollapsed] = useState(false);

  const { activeCards, doneCount, total } = useMemo(() => {
    const cards = board?.cards ?? [];
    // Exclude `rejected` cards entirely; they're neither active nor progress.
    const tracked = cards.filter(c => c.status !== 'rejected');
    const active = tracked
      .filter(c => ACTIVE_STATUSES.includes(c.status))
      .sort((a, b) => a.order - b.order);
    return {
      activeCards: active,
      doneCount: tracked.filter(c => c.status === DONE_STATUS).length,
      total: tracked.length,
    };
  }, [board]);

  // Nothing to plan → render nothing (no empty chrome above the composer).
  if (activeCards.length === 0) return null;

  return (
    <div
      className="mb-2 rounded-xl border border-line bg-surface-muted text-xs shadow-sm"
      data-testid="thread-todo-strip">
      <button
        type="button"
        onClick={() => setCollapsed(prev => !prev)}
        aria-expanded={!collapsed}
        className="flex w-full items-center gap-1.5 px-3 py-2 text-left text-content-muted transition-colors hover:text-content-secondary">
        <span aria-hidden className="text-primary-500">
          {collapsed ? '▸' : '▾'}
        </span>
        <span className="font-semibold text-content-secondary">
          {t('conversations.threadTodo.title')}
        </span>
        <span className="text-content-faint">
          {doneCount}/{total}
        </span>
      </button>

      {!collapsed && (
        // Cap the expanded list so a long plan (one card per step) can't cover
        // the latest messages/controls above the composer — scroll instead.
        <ul className="flex max-h-48 flex-col gap-0.5 overflow-y-auto px-3 pb-2 pl-5">
          {activeCards.map(card => (
            <li
              key={card.id}
              className={`flex items-start gap-1.5 break-words ${statusColorClass(card.status)}`}>
              <span aria-hidden className="font-mono">
                {statusGlyph(card.status)}
              </span>
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="min-w-0">{cardLabel(card)}</span>
                {card.status === 'blocked' && card.blocker?.trim() && (
                  // Surface why a step is stuck + what's needed next, matching
                  // the todo-tool guidance to set `blocked` with a `blocker`.
                  <span className="min-w-0 text-[11px] text-coral/80">{card.blocker.trim()}</span>
                )}
              </span>
              {card.sessionThreadId && onViewSession && (
                <button
                  type="button"
                  title={t('conversations.taskKanban.viewWork')}
                  onClick={() => onViewSession(card)}
                  className="flex-shrink-0 rounded-md border border-line px-1.5 py-0.5 text-[10px] font-medium text-content-secondary transition-colors hover:bg-surface-hover">
                  {t('conversations.taskKanban.viewWork')}
                </button>
              )}
              {card.status === 'awaiting_approval' && onDecidePlan && (
                <span className="flex flex-shrink-0 items-center gap-1">
                  <button
                    type="button"
                    title={t('chat.approval.approve')}
                    disabled={disabled}
                    onClick={() => onDecidePlan(card, true)}
                    className="rounded-md bg-ocean-600 px-1.5 py-0.5 text-[10px] font-medium text-white transition-colors hover:bg-ocean-700 disabled:opacity-40">
                    {t('chat.approval.approve')}
                  </button>
                  <button
                    type="button"
                    title={t('chat.approval.deny')}
                    disabled={disabled}
                    onClick={() => onDecidePlan(card, false)}
                    className="rounded-md border border-line px-1.5 py-0.5 text-[10px] font-medium text-content-secondary transition-colors hover:bg-surface-hover disabled:opacity-40">
                    {t('chat.approval.deny')}
                  </button>
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};

/** Prefer the card title; fall back to its objective, then a generic label. */
function cardLabel(card: TaskBoardCard): string {
  const title = card.title?.trim();
  if (title) return title;
  const objective = card.objective?.trim();
  if (objective) return objective;
  return card.id;
}

export default ThreadTodoStrip;
