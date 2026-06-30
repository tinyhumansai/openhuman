import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  LuArrowLeft,
  LuArrowRight,
  LuBot,
  LuCircleCheck,
  LuClipboardList,
  LuDatabase,
  LuExternalLink,
  LuPlay,
  LuRefreshCw,
  LuShieldCheck,
  LuWrench,
  LuX,
} from 'react-icons/lu';
import { useLocation, useNavigate } from 'react-router-dom';

import { settingsNavState } from '../../../components/settings/modal/settingsOverlay';
import Button from '../../../components/ui/Button';
import { useT } from '../../../lib/i18n/I18nContext';
import type { TaskBoard, TaskBoardCard, TaskBoardCardStatus } from '../../../types/turnState';
import {
  type FetchOutcome,
  isTauri,
  openhumanTaskSourcesFetch,
  openhumanTaskSourcesList,
  openhumanTaskSourcesStatus,
  openhumanTaskSourcesSync,
  openhumanTaskSourcesUpdate,
  type TaskSource,
  type TaskSourcesStatus,
} from '../../../utils/tauriCommands';

type ColumnDef = { status: TaskBoardCardStatus; labelKey: string };

const TASK_SOURCES_THREAD_ID = 'task-sources';

// The board surfaces five columns:
// Pending / Awaiting Approval / In Progress / Blocked / Done
const COLUMN_DEFS: ColumnDef[] = [
  { status: 'todo', labelKey: 'conversations.taskKanban.pending' },
  { status: 'awaiting_approval', labelKey: 'conversations.taskKanban.awaitingApprovalColumn' },
  { status: 'in_progress', labelKey: 'conversations.taskKanban.working' },
  { status: 'blocked', labelKey: 'conversations.taskKanban.blockedColumn' },
  { status: 'done', labelKey: 'conversations.taskKanban.done' },
];

/** The five column statuses a user can set directly from the board. */
const COLUMN_STATUSES = COLUMN_DEFS.map(column => column.status);

const STATUS_INDEX = new Map(COLUMN_DEFS.map((column, index) => [column.status, index]));

/** Per-column visual accent: left-border + background tint. Empty string = no accent. */
const COLUMN_ACCENT: Record<TaskBoardCardStatus, string> = {
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
 *  column. Drives the edit dialog's status <select>. */
const STATUS_LABEL_KEYS: Record<TaskBoardCardStatus, string> = {
  todo: 'conversations.taskKanban.pending',
  awaiting_approval: 'conversations.taskKanban.awaitingApproval',
  ready: 'conversations.taskKanban.ready',
  in_progress: 'conversations.taskKanban.working',
  blocked: 'conversations.taskKanban.blocked',
  done: 'conversations.taskKanban.done',
  rejected: 'conversations.taskKanban.rejected',
};

/** Whether a status owns a kanban column. */
function isColumnStatus(status: TaskBoardCardStatus): boolean {
  return STATUS_INDEX.has(status);
}

/** Map a card status to the column it renders under.
 *  ready → in_progress column; rejected → done column.
 *  All other statuses now own their own column. */
function columnFor(status: TaskBoardCardStatus): TaskBoardCardStatus {
  switch (status) {
    case 'ready':
      return 'in_progress';
    case 'rejected':
      return 'done';
    default:
      return status;
  }
}

interface TaskKanbanBoardProps {
  board: TaskBoard;
  disabled?: boolean;
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

export function TaskKanbanBoard({
  board,
  disabled = false,
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

  const selectedCard = useMemo(
    () => board.cards.find(card => card.id === selectedCardId) ?? null,
    [board.cards, selectedCardId]
  );
  const isTaskSourcesBoard = board.threadId === TASK_SOURCES_THREAD_ID;
  const hasSourceCards = board.cards.some(card => readSourceMetadata(card.sourceMetadata));
  const showSourceControls = isTaskSourcesBoard || hasSourceCards;

  // Always render (even with 0 cards) so a live agent board stays visible.
  const cardsByStatus = COLUMN_DEFS.reduce(
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
    const current = STATUS_INDEX.get(columnFor(card.status)) ?? 0;
    const next = COLUMN_DEFS[current + direction]?.status;
    if (!next || disabled) return;
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
    onMove?.(draggedCard, targetColumnStatus);
  };

  return (
    <div className="py-3">
      {!hideHeader && (
        <div className="mb-2 flex items-center justify-between gap-3">
          <h4 className="text-xs font-semibold uppercase tracking-wide text-content-muted">
            {t(headerTitleKey)}
          </h4>
          <div className="flex items-center gap-2">
            {showSourceControls && (
              <button
                type="button"
                aria-expanded={sourceControlsOpen}
                onClick={() => setSourceControlsOpen(open => !open)}
                className="inline-flex items-center gap-1 rounded-md border border-line px-2 py-1 text-[10px] font-medium text-content-secondary hover:bg-surface-hover">
                <LuDatabase className="h-3 w-3" />
                {t('conversations.taskKanban.sourcesButton')}
              </button>
            )}
            <span className="text-[10px] text-content-faint">{board.cards.length}</span>
          </div>
        </div>
      )}
      {showSourceControls && sourceControlsOpen && (
        <TaskSourceControls disabled={disabled} compact={!isTaskSourcesBoard} />
      )}
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-3 lg:grid-cols-5">
        {COLUMN_DEFS.map(column => {
          const cards = cardsByStatus[column.status];
          const isBlockedColumn = column.status === 'blocked';
          const isDragTarget = dragOverColumn === column.status;
          const accentClass = COLUMN_ACCENT[column.status] ?? '';
          return (
            <section
              key={column.status}
              className={`min-w-0 rounded-lg bg-surface-muted p-2 ${accentClass} ${
                isDragTarget ? 'ring-2 ring-ocean-400 bg-ocean-50/30 dark:bg-ocean-500/5' : ''
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
                  <p className="py-2 text-center text-[10px] text-content-faint dark:text-neutral-600">
                    {t('conversations.taskKanban.emptyColumn')}
                  </p>
                ) : (
                  cards.map(card => (
                    <TaskBoardArticle
                      key={card.id}
                      card={card}
                      columnStatus={column.status}
                      disabled={disabled}
                      onMove={onMove ? moveCard : undefined}
                      hasBriefActions={Boolean(onUpdateCard || onDeleteCard)}
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

function TaskBoardArticle({
  card,
  columnStatus,
  disabled,
  onMove,
  hasBriefActions,
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
    e.dataTransfer.setData('application/x-task-card-id', card.id);
    e.dataTransfer.effectAllowed = 'move';
  };

  return (
    <article
      draggable={isDraggable}
      onDragStart={isDraggable ? handleDragStart : undefined}
      className={`rounded-lg border border-line bg-surface px-2.5 py-2 shadow-sm transition-opacity ${
        mutating ? 'opacity-50' : 'opacity-100'
      } ${isDraggable ? 'cursor-grab active:cursor-grabbing' : ''}`}>
      <div className="flex items-start gap-2">
        <p className="min-w-0 flex-1 break-words text-xs font-medium leading-snug text-content">
          {card.title}
        </p>
        {card.sessionThreadId && onViewSession ? (
          <button
            type="button"
            title={t('conversations.taskKanban.viewWork')}
            onClick={() => onViewSession(card)}
            className="inline-flex flex-shrink-0 items-center gap-1 rounded-md bg-ocean-50 px-1.5 py-0.5 text-[10px] font-medium text-ocean-700 transition-colors hover:bg-ocean-100 dark:bg-ocean-500/10 dark:text-ocean-200 dark:hover:bg-ocean-500/20">
            <LuExternalLink className="h-3 w-3 flex-none" />
            {t('conversations.taskKanban.viewWork')}
          </button>
        ) : card.status === 'awaiting_approval' && onDecidePlan ? (
          <div className="flex flex-shrink-0 items-center gap-1">
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
          </div>
        ) : onWorkTask && card.status !== 'done' ? (
          <button
            type="button"
            title={t('conversations.taskKanban.workTask')}
            disabled={disabled || working}
            onClick={() => onWorkTask(card)}
            className="inline-flex flex-shrink-0 items-center gap-1 rounded-md bg-ocean-600 px-1.5 py-0.5 text-[10px] font-medium text-white transition-colors hover:bg-ocean-700 disabled:opacity-40">
            <LuPlay className="h-3 w-3" />
            {working
              ? t('conversations.taskKanban.startingTask')
              : t('conversations.taskKanban.workTask')}
          </button>
        ) : onMove && isColumnStatus(columnFor(card.status)) ? (
          <div className="flex flex-shrink-0 items-center gap-0.5">
            <button
              type="button"
              title={t('conversations.taskKanban.moveLeft')}
              aria-label={t('conversations.taskKanban.moveLeft')}
              disabled={disabled || columnStatus === 'todo'}
              onClick={() => onMove(card, -1)}
              className="flex h-7 w-7 items-center justify-center rounded-md text-content-faint transition-colors hover:bg-surface-hover dark:bg-surface-muted hover:text-content-secondary dark:text-neutral-200 disabled:opacity-25">
              <LuArrowLeft className="h-4 w-4" />
            </button>
            <button
              type="button"
              title={t('conversations.taskKanban.moveRight')}
              aria-label={t('conversations.taskKanban.moveRight')}
              disabled={disabled || columnStatus === 'done'}
              onClick={() => onMove(card, 1)}
              className="flex h-7 w-7 items-center justify-center rounded-md text-content-faint transition-colors hover:bg-surface-hover dark:bg-surface-muted hover:text-content-secondary dark:text-neutral-200 disabled:opacity-25">
              <LuArrowRight className="h-4 w-4" />
            </button>
          </div>
        ) : null}
      </div>
      <div className="mt-2 flex flex-wrap gap-1.5">
        {card.assignedAgent && (
          <span className="inline-flex max-w-full items-center gap-1 rounded-md bg-ocean-50 px-1.5 py-0.5 text-[10px] text-ocean-700 dark:bg-ocean-500/10 dark:text-ocean-200">
            <LuBot className="h-3 w-3 flex-none" />
            <span className="truncate">{card.assignedAgent}</span>
          </span>
        )}
        {card.allowedTools && card.allowedTools.length > 0 && (
          <span className="inline-flex items-center gap-1 rounded-md bg-surface-subtle px-1.5 py-0.5 text-[10px] text-content-secondary">
            <LuWrench className="h-3 w-3" />
            {card.allowedTools.length}
          </span>
        )}
        {source && (
          <span className="inline-flex max-w-full items-center gap-1 rounded-md bg-sky-50 px-1.5 py-0.5 text-[10px] text-sky-700 dark:bg-sky-500/10 dark:text-sky-200">
            <LuDatabase className="h-3 w-3 flex-none" />
            <span className="truncate">{sourceBadgeLabel(source, t)}</span>
          </span>
        )}
        {source?.url && (
          <a
            href={source.url}
            target="_blank"
            rel="noreferrer"
            title={t('conversations.taskKanban.source.openExternal')}
            className="inline-flex items-center gap-1 rounded-md bg-surface-subtle px-1.5 py-0.5 text-[10px] text-content-secondary hover:bg-surface-strong dark:hover:bg-neutral-700">
            <LuExternalLink className="h-3 w-3" />
            {t('conversations.taskKanban.source.openExternalShort')}
          </a>
        )}
        {card.approvalMode && (
          <span className="inline-flex items-center gap-1 rounded-md bg-amber-50 px-1.5 py-0.5 text-[10px] text-amber-700 dark:bg-amber-500/10 dark:text-amber-200">
            <LuShieldCheck className="h-3 w-3" />
            {card.approvalMode === 'required'
              ? t('conversations.taskKanban.approval.requiredBadge')
              : t('conversations.taskKanban.approval.notRequiredBadge')}
          </span>
        )}
        {card.acceptanceCriteria && card.acceptanceCriteria.length > 0 && (
          <span className="inline-flex items-center gap-1 rounded-md bg-sage-50 px-1.5 py-0.5 text-[10px] text-sage-700 dark:bg-sage-500/10 dark:text-sage-200">
            <LuCircleCheck className="h-3 w-3" />
            {card.acceptanceCriteria.length}
          </span>
        )}
        {/* Status badge: ready → "Ready to start" (sage); rejected → "Rejected" (coral) */}
        {card.status === 'ready' && (
          <span className="inline-flex items-center gap-1 rounded-md bg-sage-50 px-1.5 py-0.5 text-[10px] text-sage-700 dark:bg-sage-500/10 dark:text-sage-200">
            {t('conversations.taskKanban.statusBadge.ready')}
          </span>
        )}
        {card.status === 'rejected' && (
          <span className="inline-flex items-center gap-1 rounded-md bg-coral-50 px-1.5 py-0.5 text-[10px] text-coral-700 dark:bg-coral-500/10 dark:text-coral-200">
            {t('conversations.taskKanban.statusBadge.rejected')}
          </span>
        )}
        {/* Evidence badge: shown on the card when evidence is present */}
        {card.evidence && card.evidence.length > 0 && (
          <span className="inline-flex items-center gap-1 rounded-md bg-sky-50 px-1.5 py-0.5 text-[10px] text-sky-700 dark:bg-sky-500/10 dark:text-sky-200">
            {t('conversations.taskKanban.evidenceBadge').replace(
              '{count}',
              String(card.evidence.length)
            )}
          </span>
        )}
      </div>
      {card.objective && (
        <p className="mt-1 break-words text-[11px] leading-snug text-content-muted">
          {card.objective}
        </p>
      )}
      {card.notes && (
        <p className="mt-1 break-words text-[11px] leading-snug text-content-muted">{card.notes}</p>
      )}
      {/* Blocker text: always shown for blocked cards (column or status) */}
      {card.blocker && (card.status === 'blocked' || columnStatus === 'blocked') && (
        <p className="mt-1 break-words text-[11px] leading-snug text-coral-600">{card.blocker}</p>
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
        <button
          type="button"
          onClick={onOpenBrief}
          className="mt-2 inline-flex items-center gap-1 text-[11px] font-medium text-ocean-600 hover:text-ocean-700 dark:text-ocean-300 dark:hover:text-ocean-200">
          <LuClipboardList className="h-3 w-3" />
          {t('conversations.taskKanban.briefButton')}
        </button>
      )}
    </article>
  );
}

interface TaskSourceMetadata {
  provider?: string;
  sourceId?: string;
  externalId?: string;
  url?: string;
  repo?: string;
  urgency?: number;
}

function readSourceMetadata(
  value: Record<string, unknown> | null | undefined
): TaskSourceMetadata | null {
  if (!value || typeof value !== 'object') return null;
  const provider = readString(value.provider);
  const sourceId = readString(value.source_id) ?? readString(value.sourceId);
  const externalId = readString(value.external_id) ?? readString(value.externalId);
  const url = readString(value.url);
  const repo = readString(value.repo);
  const urgency = readNumber(value.urgency);
  if (!provider && !sourceId && !externalId && !url && !repo && urgency === undefined) {
    return null;
  }
  return { provider, sourceId, externalId, url, repo, urgency };
}

function readString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined;
}

function readNumber(value: unknown): number | undefined {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string') {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return undefined;
}

function providerLabel(provider: string | undefined, t: (key: string) => string): string {
  switch (provider) {
    case 'github':
      return t('settings.taskSources.providers.github');
    case 'notion':
      return t('settings.taskSources.providers.notion');
    case 'linear':
      return t('settings.taskSources.providers.linear');
    case 'clickup':
      return t('settings.taskSources.providers.clickup');
    default:
      return provider ?? t('conversations.taskKanban.source.unknownProvider');
  }
}

function sourceBadgeLabel(source: TaskSourceMetadata, t: (key: string) => string): string {
  const provider = providerLabel(source.provider, t);
  if (source.repo && source.externalId) return `${provider} · ${source.repo}#${source.externalId}`;
  if (source.externalId) return `${provider} · ${source.externalId}`;
  return provider;
}

function formatUrgency(
  urgency: number | undefined,
  t: (key: string) => string
): string | undefined {
  if (urgency === undefined) return undefined;
  const percent = Math.round(Math.max(0, Math.min(1, urgency)) * 100);
  return t('conversations.taskKanban.source.urgencyValue').replace('{percent}', String(percent));
}

function formatFetchNotice(outcome: FetchOutcome, t: (key: string) => string): string {
  return t('settings.taskSources.fetchResult')
    .replace('{routed}', String(outcome.routed))
    .replace('{fetched}', String(outcome.fetched))
    .replace('{pruned}', String(outcome.pruned ?? 0));
}

function formatSyncNotice(outcomes: FetchOutcome[], t: (key: string) => string): string {
  const totals = outcomes.reduce(
    (acc, outcome) => ({
      fetched: acc.fetched + outcome.fetched,
      routed: acc.routed + outcome.routed,
      pruned: acc.pruned + (outcome.pruned ?? 0),
    }),
    { fetched: 0, routed: 0, pruned: 0 }
  );
  return t('settings.taskSources.fetchResult')
    .replace('{routed}', String(totals.routed))
    .replace('{fetched}', String(totals.fetched))
    .replace('{pruned}', String(totals.pruned));
}

function TaskSourceControls({ disabled, compact }: { disabled: boolean; compact: boolean }) {
  const { t } = useT();
  const navigate = useNavigate();
  const location = useLocation();
  const [loading, setLoading] = useState(true);
  const [sources, setSources] = useState<TaskSource[]>([]);
  const [status, setStatus] = useState<TaskSourcesStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!isTauri()) {
      setLoading(false);
      setError(t('conversations.taskKanban.sources.desktopOnly'));
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const [nextSources, nextStatus] = await Promise.all([
        openhumanTaskSourcesList(),
        openhumanTaskSourcesStatus(),
      ]);
      setSources(nextSources);
      setStatus(nextStatus);
    } catch (err) {
      setError(
        `${t('settings.taskSources.loadError')}: ${err instanceof Error ? err.message : String(err)}`
      );
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    const id = window.setTimeout(() => {
      void load();
    }, 0);
    return () => window.clearTimeout(id);
  }, [load]);

  const toggleSource = async (source: TaskSource) => {
    if (busyKey) return;
    setBusyKey(`toggle:${source.id}`);
    setError(null);
    setNotice(null);
    try {
      const updated = await openhumanTaskSourcesUpdate(source.id, { enabled: !source.enabled });
      setSources(prev => prev.map(item => (item.id === updated.id ? updated : item)));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyKey(null);
    }
  };

  const fetchSource = async (source: TaskSource) => {
    if (busyKey) return;
    setBusyKey(`fetch:${source.id}`);
    setError(null);
    setNotice(null);
    try {
      const outcome = await openhumanTaskSourcesFetch(source.id);
      await load();
      if (outcome.error) {
        setError(outcome.error);
      } else {
        setNotice(formatFetchNotice(outcome, t));
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyKey(null);
    }
  };

  const syncSources = async () => {
    if (busyKey) return;
    setBusyKey('sync');
    setError(null);
    setNotice(null);
    try {
      const outcomes = await openhumanTaskSourcesSync();
      await load();
      const firstError = outcomes.find(outcome => outcome.error)?.error;
      if (firstError) {
        setError(firstError);
      } else {
        setNotice(formatSyncNotice(outcomes, t));
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyKey(null);
    }
  };

  return (
    <section className="mb-3 rounded-lg border border-line bg-surface p-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="min-w-0">
          <h5 className="text-xs font-semibold text-content">
            {t('conversations.taskKanban.sources.title')}
          </h5>
          {!compact && status && (
            <p className="text-[11px] text-content-muted">
              {status.enabled
                ? t('conversations.taskKanban.sources.statusEnabled')
                : t('settings.taskSources.disabledBanner')}
            </p>
          )}
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => navigate('/settings/integrations', settingsNavState(location))}
            className="text-[11px] font-medium text-ocean-600 hover:text-ocean-700 dark:text-ocean-300 dark:hover:text-ocean-200">
            {t('conversations.taskKanban.sources.manage')}
          </button>
          <button
            type="button"
            disabled={disabled || loading || busyKey !== null || sources.length === 0}
            onClick={() => void syncSources()}
            className="inline-flex items-center gap-1 rounded-md border border-line px-2 py-1 text-[11px] font-medium text-content-secondary hover:bg-surface-hover disabled:opacity-40">
            <LuRefreshCw className="h-3 w-3" />
            {busyKey === 'sync'
              ? t('settings.taskSources.syncing')
              : t('settings.taskSources.syncAll')}
          </button>
          <button
            type="button"
            aria-label={t('settings.taskSources.refresh')}
            disabled={loading}
            onClick={() => void load()}
            className="flex h-7 w-7 items-center justify-center rounded-md border border-line text-content-muted hover:bg-surface-hover disabled:opacity-40 dark:text-content-secondary">
            <LuRefreshCw className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
      {error && (
        <p className="mt-2 rounded-md bg-coral-50 px-2 py-1.5 text-[11px] text-coral-700 dark:bg-coral-500/10 dark:text-coral-200">
          {error}
        </p>
      )}
      {notice && (
        <p className="mt-2 rounded-md bg-sky-50 px-2 py-1.5 text-[11px] text-sky-700 dark:bg-sky-500/10 dark:text-sky-200">
          {notice}
        </p>
      )}
      {loading ? (
        <p className="mt-2 text-[11px] text-content-faint">{t('common.loading')}</p>
      ) : sources.length === 0 ? (
        <p className="mt-2 text-[11px] text-content-faint">{t('settings.taskSources.empty')}</p>
      ) : (
        <ul className="mt-3 grid gap-2 sm:grid-cols-2">
          {sources.map(source => (
            <li key={source.id} className="min-w-0 rounded-lg border border-line px-2.5 py-2">
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <p className="truncate text-xs font-medium text-content">
                    {source.name || providerLabel(source.provider, t)}
                  </p>
                  <p className="truncate text-[11px] text-content-muted">
                    {providerLabel(source.provider, t)}
                    {source.target === 'agent_todo_proactive'
                      ? ` · ${t('settings.taskSources.proactive')}`
                      : ''}
                  </p>
                </div>
                <span
                  className={`flex-none rounded-md px-1.5 py-0.5 text-[10px] ${
                    source.enabled
                      ? 'bg-sage-50 text-sage-700 dark:bg-sage-500/10 dark:text-sage-200'
                      : 'bg-surface-subtle text-content-muted'
                  }`}>
                  {source.enabled
                    ? t('settings.taskSources.statusEnabled')
                    : t('settings.taskSources.statusDisabled')}
                </span>
              </div>
              <div className="mt-2 flex flex-wrap gap-1.5">
                <button
                  type="button"
                  disabled={disabled || busyKey !== null}
                  onClick={() => void fetchSource(source)}
                  className="inline-flex items-center gap-1 rounded-md border border-line px-2 py-1 text-[11px] font-medium text-content-secondary hover:bg-surface-hover disabled:opacity-40">
                  <LuRefreshCw className="h-3 w-3" />
                  {busyKey === `fetch:${source.id}`
                    ? t('settings.taskSources.fetching')
                    : t('settings.taskSources.fetchNow')}
                </button>
                <button
                  type="button"
                  disabled={disabled || busyKey !== null}
                  onClick={() => void toggleSource(source)}
                  className="rounded-md border border-line px-2 py-1 text-[11px] font-medium text-content-secondary hover:bg-surface-hover disabled:opacity-40">
                  {source.enabled
                    ? t('settings.taskSources.disable')
                    : t('settings.taskSources.enable')}
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function TaskBriefDialog({
  card,
  disabled,
  onClose,
  onUpdate,
  onDelete,
}: {
  card: TaskBoardCard;
  disabled: boolean;
  onClose: () => void;
  onUpdate?: (card: TaskBoardCard, nextCard: TaskBoardCard) => void;
  onDelete?: (card: TaskBoardCard) => void;
}) {
  const { t } = useT();
  const source = readSourceMetadata(card.sourceMetadata);
  const editable = Boolean(onUpdate) && !disabled;
  const deletable = Boolean(onDelete) && !disabled;

  const handleDelete = () => {
    if (!deletable) return;
    onDelete?.(card);
    onClose();
  };
  const [title, setTitle] = useState(card.title);
  const [status, setStatus] = useState<TaskBoardCardStatus>(card.status);
  const [objective, setObjective] = useState(card.objective ?? '');
  const [assignedAgent, setAssignedAgent] = useState(card.assignedAgent ?? '');
  const [approvalMode, setApprovalMode] = useState(card.approvalMode ?? '');
  const [plan, setPlan] = useState(joinLines(card.plan));
  const [allowedTools, setAllowedTools] = useState(joinLines(card.allowedTools));
  const [acceptanceCriteria, setAcceptanceCriteria] = useState(joinLines(card.acceptanceCriteria));
  const [evidence, setEvidence] = useState(joinLines(card.evidence));
  const [notes, setNotes] = useState(card.notes ?? '');
  const [blocker, setBlocker] = useState(card.blocker ?? '');

  const save = () => {
    if (!editable) return;
    const trimmedTitle = title.trim();
    if (!trimmedTitle) return;
    onUpdate?.(card, {
      ...card,
      title: trimmedTitle,
      status,
      objective: emptyToNull(objective),
      assignedAgent: emptyToNull(assignedAgent),
      approvalMode:
        approvalMode === 'required' || approvalMode === 'not_required' ? approvalMode : null,
      plan: splitLines(plan),
      allowedTools: splitLines(allowedTools),
      acceptanceCriteria: splitLines(acceptanceCriteria),
      evidence: splitLines(evidence),
      notes: emptyToNull(notes),
      blocker: emptyToNull(blocker),
    });
    onClose();
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 px-4 py-6">
      <section className="max-h-full w-full max-w-xl overflow-y-auto rounded-lg border border-line bg-surface p-4 shadow-xl">
        <div className="mb-3 flex items-start justify-between gap-3">
          <div className="min-w-0">
            <p className="text-[11px] font-semibold uppercase text-content-faint">
              {t('conversations.taskKanban.briefTitle')}
            </p>
            <h3 className="break-words text-base font-semibold text-content">{card.title}</h3>
          </div>
          <Button
            iconOnly
            variant="tertiary"
            size="sm"
            aria-label={t('conversations.taskKanban.closeBrief')}
            onClick={onClose}
            className="flex-none">
            <LuX className="h-4 w-4" />
          </Button>
        </div>

        {source && <SourceBrief source={source} />}

        {editable ? (
          <div className="space-y-3 text-sm">
            <label className="block">
              <span className="mb-1 block text-xs font-semibold text-content-muted">
                {t('conversations.taskKanban.field.title')}
              </span>
              <input
                value={title}
                onChange={e => setTitle(e.target.value)}
                className="w-full rounded-md border border-line bg-surface px-2 py-1.5 text-sm text-content dark:bg-surface-canvas"
              />
            </label>
            <div className="grid gap-3 sm:grid-cols-3">
              <label className="block">
                <span className="mb-1 block text-xs font-semibold text-content-muted">
                  {t('conversations.taskKanban.field.status')}
                </span>
                <select
                  value={status}
                  onChange={e => setStatus(e.target.value as TaskBoardCardStatus)}
                  className="w-full rounded-md border border-line bg-surface px-2 py-1.5 text-sm text-content dark:bg-surface-canvas">
                  {(COLUMN_STATUSES.includes(status)
                    ? COLUMN_STATUSES
                    : [status, ...COLUMN_STATUSES]
                  ).map(s => (
                    <option key={s} value={s}>
                      {t(STATUS_LABEL_KEYS[s])}
                    </option>
                  ))}
                </select>
              </label>
              <BriefInput
                label={t('conversations.taskKanban.field.assignedAgent')}
                value={assignedAgent}
                onChange={setAssignedAgent}
              />
              <label className="block">
                <span className="mb-1 block text-xs font-semibold text-content-muted">
                  {t('conversations.taskKanban.field.approval')}
                </span>
                <select
                  value={approvalMode}
                  onChange={e => setApprovalMode(e.target.value)}
                  className="w-full rounded-md border border-line bg-surface px-2 py-1.5 text-sm text-content dark:bg-surface-canvas">
                  <option value="">{t('conversations.taskKanban.approval.default')}</option>
                  <option value="required">
                    {t('conversations.taskKanban.approval.required')}
                  </option>
                  <option value="not_required">
                    {t('conversations.taskKanban.approval.notRequired')}
                  </option>
                </select>
              </label>
            </div>
            <BriefInput
              label={t('conversations.taskKanban.field.objective')}
              value={objective}
              onChange={setObjective}
            />
            <BriefTextarea
              label={t('conversations.taskKanban.field.plan')}
              value={plan}
              onChange={setPlan}
            />
            <BriefTextarea
              label={t('conversations.taskKanban.field.allowedTools')}
              value={allowedTools}
              onChange={setAllowedTools}
            />
            <BriefTextarea
              label={t('conversations.taskKanban.field.acceptanceCriteria')}
              value={acceptanceCriteria}
              onChange={setAcceptanceCriteria}
            />
            <BriefTextarea
              label={t('conversations.taskKanban.field.evidence')}
              value={evidence}
              onChange={setEvidence}
            />
            <BriefTextarea
              label={t('conversations.taskKanban.field.notes')}
              value={notes}
              onChange={setNotes}
            />
            <BriefTextarea
              label={t('conversations.taskKanban.field.blocker')}
              value={blocker}
              onChange={setBlocker}
            />
            <div className="flex items-center justify-between gap-2 pt-1">
              {deletable ? (
                <Button variant="secondary" tone="danger" size="sm" onClick={handleDelete}>
                  {t('conversations.taskKanban.deleteCard')}
                </Button>
              ) : (
                <span />
              )}
              <div className="flex gap-2">
                <Button variant="secondary" size="sm" onClick={onClose}>
                  {t('common.cancel')}
                </Button>
                <Button variant="primary" size="sm" onClick={save} disabled={!title.trim()}>
                  {t('conversations.taskKanban.saveChanges')}
                </Button>
              </div>
            </div>
          </div>
        ) : (
          <div className="space-y-4 text-sm">
            <BriefText
              label={t('conversations.taskKanban.field.objective')}
              value={card.objective}
            />
            <BriefText
              label={t('conversations.taskKanban.field.assignedAgent')}
              value={card.assignedAgent}
              mono
            />
            <BriefText
              label={t('conversations.taskKanban.field.approval')}
              value={
                card.approvalMode === 'required'
                  ? t('conversations.taskKanban.approval.requiredBeforeExecution')
                  : card.approvalMode === 'not_required'
                    ? t('conversations.taskKanban.approval.notRequired')
                    : undefined
              }
            />
            <BriefList
              label={t('conversations.taskKanban.field.plan')}
              values={card.plan}
              ordered
            />
            <BriefList
              label={t('conversations.taskKanban.field.allowedTools')}
              values={card.allowedTools}
              mono
            />
            <BriefList
              label={t('conversations.taskKanban.field.acceptanceCriteria')}
              values={card.acceptanceCriteria}
            />
            <BriefList
              label={t('conversations.taskKanban.field.evidence')}
              values={card.evidence}
            />
            <BriefText label={t('conversations.taskKanban.field.notes')} value={card.notes} />
            <BriefText
              label={t('conversations.taskKanban.field.blocker')}
              value={card.blocker}
              tone="danger"
            />
            {deletable && (
              <div className="flex justify-end pt-1">
                <Button variant="secondary" tone="danger" size="sm" onClick={handleDelete}>
                  {t('conversations.taskKanban.deleteCard')}
                </Button>
              </div>
            )}
          </div>
        )}
      </section>
    </div>
  );
}

function SourceBrief({ source }: { source: TaskSourceMetadata }) {
  const { t } = useT();
  const urgency = formatUrgency(source.urgency, t);

  return (
    <div className="mb-4 rounded-lg border border-sky-200 bg-sky-50 p-3 text-sm dark:border-sky-500/20 dark:bg-sky-500/10">
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
        <h4 className="text-xs font-semibold text-sky-800 dark:text-sky-100">
          {t('conversations.taskKanban.source.title')}
        </h4>
        {source.url && (
          <a
            href={source.url}
            target="_blank"
            rel="noreferrer"
            className="inline-flex items-center gap-1 text-xs font-medium text-ocean-600 hover:text-ocean-700 dark:text-ocean-300 dark:hover:text-ocean-200">
            <LuExternalLink className="h-3 w-3" />
            {t('conversations.taskKanban.source.openExternal')}
          </a>
        )}
      </div>
      <dl className="grid gap-2 sm:grid-cols-2">
        <SourceBriefField
          label={t('settings.taskSources.provider')}
          value={providerLabel(source.provider, t)}
        />
        <SourceBriefField
          label={t('conversations.taskKanban.source.sourceId')}
          value={source.sourceId}
          mono
        />
        <SourceBriefField
          label={t('conversations.taskKanban.source.externalId')}
          value={source.externalId}
          mono
        />
        <SourceBriefField label={t('conversations.taskKanban.source.repo')} value={source.repo} />
        <SourceBriefField label={t('conversations.taskKanban.source.urgency')} value={urgency} />
      </dl>
    </div>
  );
}

function SourceBriefField({
  label,
  value,
  mono = false,
}: {
  label: string;
  value?: string;
  mono?: boolean;
}) {
  if (!value) return null;
  return (
    <div className="min-w-0">
      <dt className="text-[11px] font-semibold text-sky-700 dark:text-sky-200">{label}</dt>
      <dd className={`mt-0.5 break-words text-xs text-content ${mono ? 'font-mono' : ''}`}>
        {value}
      </dd>
    </div>
  );
}

function BriefInput({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <label className="block">
      <span className="mb-1 block text-xs font-semibold text-content-muted">{label}</span>
      <input
        value={value}
        onChange={e => onChange(e.target.value)}
        className="w-full rounded-md border border-line bg-surface px-2 py-1.5 text-sm text-content dark:bg-surface-canvas"
      />
    </label>
  );
}

function BriefTextarea({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <label className="block">
      <span className="mb-1 block text-xs font-semibold text-content-muted">{label}</span>
      <textarea
        value={value}
        onChange={e => onChange(e.target.value)}
        rows={3}
        className="w-full resize-y rounded-md border border-line bg-surface px-2 py-1.5 text-sm text-content dark:bg-surface-canvas"
      />
    </label>
  );
}

function BriefText({
  label,
  value,
  mono = false,
  tone = 'default',
}: {
  label: string;
  value?: string | null;
  mono?: boolean;
  tone?: 'default' | 'danger';
}) {
  if (!value) return null;
  return (
    <div>
      <h4 className="mb-1 text-xs font-semibold text-content-muted">{label}</h4>
      <p
        className={`break-words text-sm ${
          mono ? 'font-mono' : ''
        } ${tone === 'danger' ? 'text-coral-600' : 'text-content'}`}>
        {value}
      </p>
    </div>
  );
}

function BriefList({
  label,
  values,
  ordered = false,
  mono = false,
}: {
  label: string;
  values?: string[];
  ordered?: boolean;
  mono?: boolean;
}) {
  if (!values?.length) return null;
  const List = ordered ? 'ol' : 'ul';
  return (
    <div>
      <h4 className="mb-1 text-xs font-semibold text-content-muted">{label}</h4>
      <List
        className={`space-y-1 ${
          ordered ? 'list-decimal' : 'list-disc'
        } list-inside text-sm text-content ${mono ? 'font-mono' : ''}`}>
        {values.map((value, index) => (
          <li key={index} className="break-words">
            {value}
          </li>
        ))}
      </List>
    </div>
  );
}

function joinLines(values?: string[]): string {
  return values?.join('\n') ?? '';
}

function splitLines(value: string): string[] {
  return value
    .split('\n')
    .map(line => line.trim())
    .filter(Boolean);
}

function emptyToNull(value: string): string | null {
  const trimmed = value.trim();
  return trimmed ? trimmed : null;
}
