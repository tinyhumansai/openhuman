/**
 * Column layout and status→column mapping for the task board.
 *
 * Extracted verbatim out of `TaskKanbanBoard.tsx` when that file was split, so
 * the board, the card and the brief editor can each reach the same table
 * without importing one another.
 */
import type { TaskBoardCardStatus } from '../../../types/turnState';

export type ColumnDef = { status: TaskBoardCardStatus; labelKey: string };

/**
 * Default board layout — five columns:
 * Pending / Awaiting Approval / In Progress / Blocked / Done.
 * Callers can pass their own `columns` (e.g. the orchestrator's 4-column board).
 */
export const COLUMN_DEFS: ColumnDef[] = [
  { status: 'todo', labelKey: 'conversations.taskKanban.pending' },
  { status: 'awaiting_approval', labelKey: 'conversations.taskKanban.awaitingApprovalColumn' },
  { status: 'in_progress', labelKey: 'conversations.taskKanban.working' },
  { status: 'blocked', labelKey: 'conversations.taskKanban.blockedColumn' },
  { status: 'done', labelKey: 'conversations.taskKanban.done' },
];

/** Statuses offered in the edit dialog's status dropdown (the default set). */
export const COLUMN_STATUSES = COLUMN_DEFS.map(column => column.status);

/** Per-column visual accent: left-border + background tint. Empty string = no accent. */
export const COLUMN_ACCENT: Record<TaskBoardCardStatus, string> = {
  todo: '',
  awaiting_approval:
    'border-l-2 border-l-amber-400 bg-amber-50/60 dark:bg-amber-500/5 dark:border-l-amber-500/60',
  in_progress: '',
  blocked:
    'border-l-2 border-l-coral-400 bg-coral-50/60 dark:bg-coral-500/5 dark:border-l-coral-500/60',
  done: '',
  ready: '',
  rejected: '',
};

/** Label key for *every* status, including statuses that don't own a kanban
 *  column. Drives the edit dialog's status `<select>`. */
export const STATUS_LABEL_KEYS: Record<TaskBoardCardStatus, string> = {
  todo: 'conversations.taskKanban.pending',
  awaiting_approval: 'conversations.taskKanban.awaitingApproval',
  ready: 'conversations.taskKanban.ready',
  in_progress: 'conversations.taskKanban.working',
  blocked: 'conversations.taskKanban.blocked',
  done: 'conversations.taskKanban.done',
  rejected: 'conversations.taskKanban.rejected',
};

/** Default status→column index, over the full column set. Used by the shared
 *  status helpers below (a caller's custom `columns` are always a subset). */
const STATUS_INDEX = new Map(COLUMN_DEFS.map((column, index) => [column.status, index]));

/** Whether a status owns a kanban column (in the default set). */
export function isColumnStatus(status: TaskBoardCardStatus): boolean {
  return STATUS_INDEX.has(status);
}

/** Map a card status to the column it renders under.
 *  ready → in_progress column; rejected → done column; else its own status. */
export function columnFor(status: TaskBoardCardStatus): TaskBoardCardStatus {
  switch (status) {
    case 'ready':
      return 'in_progress';
    case 'rejected':
      return 'done';
    default:
      return status;
  }
}
