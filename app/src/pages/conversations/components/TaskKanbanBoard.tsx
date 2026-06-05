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
import { useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import type { TaskBoard, TaskBoardCard, TaskBoardCardStatus } from '../../../types/turnState';
import {
  type FetchOutcome,
  isTauri,
  openhumanTaskSourcesFetch,
  openhumanTaskSourcesList,
  openhumanTaskSourcesStatus,
  openhumanTaskSourcesUpdate,
  type TaskSource,
  type TaskSourcesStatus,
} from '../../../utils/tauriCommands';

type ColumnDef = { status: TaskBoardCardStatus; labelKey: string };

const TASK_SOURCES_THREAD_ID = 'task-sources';

// The board surfaces exactly three columns — Pending / Working / Done. The
// richer status set the core tracks (approval flow, blocked, rejected) is
// bucketed into these three via `columnFor`.
const COLUMN_DEFS: ColumnDef[] = [
  { status: 'todo', labelKey: 'conversations.taskKanban.pending' },
  { status: 'in_progress', labelKey: 'conversations.taskKanban.working' },
  { status: 'done', labelKey: 'conversations.taskKanban.done' },
];

/** The three statuses a user can set directly from the board. */
const COLUMN_STATUSES = COLUMN_DEFS.map(column => column.status);

const STATUS_INDEX = new Map(COLUMN_DEFS.map((column, index) => [column.status, index]));

/** Label key for *every* status, including the approval-flow statuses that
 *  don't own a kanban column. Drives the edit dialog's status `<select>` so a
 *  card whose status is `awaiting_approval`/`ready`/`rejected` renders a
 *  matching option instead of a controlled-select value with no option (which
 *  React warns about and which renders as the first option, hiding the real
 *  status from the user). */
const STATUS_LABEL_KEYS: Record<TaskBoardCardStatus, string> = {
  todo: 'conversations.taskKanban.pending',
  awaiting_approval: 'conversations.taskKanban.awaitingApproval',
  ready: 'conversations.taskKanban.ready',
  in_progress: 'conversations.taskKanban.working',
  blocked: 'conversations.taskKanban.blocked',
  done: 'conversations.taskKanban.done',
  rejected: 'conversations.taskKanban.rejected',
};

/** Whether a status owns a kanban column (vs the approval-flow / terminal
 *  statuses that are bucketed into an existing column). */
function isColumnStatus(status: TaskBoardCardStatus): boolean {
  return STATUS_INDEX.has(status);
}

/** Map a card status to the column it renders under. Pre-execution approval
 *  statuses sit in `Pending`; `blocked` and `rejected` are surfaced under
 *  `Done` so the board stays a clean three-column Pending / Working / Done. */
function columnFor(status: TaskBoardCardStatus): TaskBoardCardStatus {
  switch (status) {
    case 'awaiting_approval':
    case 'ready':
      return 'todo';
    case 'blocked':
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
}: TaskKanbanBoardProps) {
  const { t } = useT();
  const [selectedCardId, setSelectedCardId] = useState<string | null>(null);
  const [sourceControlsOpen, setSourceControlsOpen] = useState(false);
  const selectedCard = useMemo(
    () => board.cards.find(card => card.id === selectedCardId) ?? null,
    [board.cards, selectedCardId]
  );
  const isTaskSourcesBoard = board.threadId === TASK_SOURCES_THREAD_ID;
  const hasSourceCards = board.cards.some(card => readSourceMetadata(card.sourceMetadata));
  const showSourceControls = isTaskSourcesBoard || hasSourceCards;

  if (board.cards.length === 0 && !isTaskSourcesBoard) return null;

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
    const current = STATUS_INDEX.get(card.status) ?? 0;
    const next = COLUMN_DEFS[current + direction]?.status;
    if (!next || disabled) return;
    onMove?.(card, next);
  };

  return (
    <div className="py-3">
      {!hideHeader && (
        <div className="mb-2 flex items-center justify-between gap-3">
          <h4 className="text-xs font-semibold uppercase tracking-wide text-stone-500 dark:text-neutral-400">
            {t(headerTitleKey)}
          </h4>
          <div className="flex items-center gap-2">
            {showSourceControls && (
              <button
                type="button"
                aria-expanded={sourceControlsOpen}
                onClick={() => setSourceControlsOpen(open => !open)}
                className="inline-flex items-center gap-1 rounded-md border border-stone-200 px-2 py-1 text-[10px] font-medium text-stone-600 hover:bg-stone-50 dark:border-neutral-800 dark:text-neutral-300 dark:hover:bg-neutral-800">
                <LuDatabase className="h-3 w-3" />
                {t('conversations.taskKanban.sourcesButton')}
              </button>
            )}
            <span className="text-[10px] text-stone-400 dark:text-neutral-500">
              {board.cards.length}
            </span>
          </div>
        </div>
      )}
      {showSourceControls && sourceControlsOpen && (
        <TaskSourceControls disabled={disabled} compact={!isTaskSourcesBoard} />
      )}
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
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
                  onOpenBrief={() => setSelectedCardId(card.id)}
                />
              ))}
            </div>
          </section>
        ))}
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
  onOpenBrief: () => void;
}) {
  const { t } = useT();
  const source = readSourceMetadata(card.sourceMetadata);

  return (
    <article className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-2.5 py-2 shadow-sm">
      <div className="flex items-start gap-2">
        <p className="min-w-0 flex-1 break-words text-xs font-medium leading-snug text-stone-800 dark:text-neutral-100">
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
              className="rounded-md border border-stone-200 px-1.5 py-0.5 text-[10px] font-medium text-stone-600 transition-colors hover:bg-stone-100 disabled:opacity-40 dark:border-neutral-700 dark:text-neutral-300 dark:hover:bg-neutral-800">
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
        ) : onMove && isColumnStatus(card.status) ? (
          <div className="flex flex-shrink-0 items-center gap-0.5">
            <button
              type="button"
              title={t('conversations.taskKanban.moveLeft')}
              aria-label={t('conversations.taskKanban.moveLeft')}
              disabled={disabled || columnStatus === 'todo'}
              onClick={() => onMove(card, -1)}
              className="flex h-5 w-5 items-center justify-center rounded-md text-stone-400 dark:text-neutral-500 transition-colors hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800 hover:text-stone-700 dark:hover:text-neutral-200 dark:text-neutral-200 disabled:opacity-25">
              <LuArrowLeft className="h-3 w-3" />
            </button>
            <button
              type="button"
              title={t('conversations.taskKanban.moveRight')}
              aria-label={t('conversations.taskKanban.moveRight')}
              disabled={disabled || columnStatus === 'done'}
              onClick={() => onMove(card, 1)}
              className="flex h-5 w-5 items-center justify-center rounded-md text-stone-400 dark:text-neutral-500 transition-colors hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800 hover:text-stone-700 dark:hover:text-neutral-200 dark:text-neutral-200 disabled:opacity-25">
              <LuArrowRight className="h-3 w-3" />
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
          <span className="inline-flex items-center gap-1 rounded-md bg-stone-100 px-1.5 py-0.5 text-[10px] text-stone-600 dark:bg-neutral-800 dark:text-neutral-300">
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
            className="inline-flex items-center gap-1 rounded-md bg-stone-100 px-1.5 py-0.5 text-[10px] text-stone-600 hover:bg-stone-200 dark:bg-neutral-800 dark:text-neutral-300 dark:hover:bg-neutral-700">
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
      </div>
      {card.objective && (
        <p className="mt-1 break-words text-[11px] leading-snug text-stone-500 dark:text-neutral-400">
          {card.objective}
        </p>
      )}
      {card.notes && (
        <p className="mt-1 break-words text-[11px] leading-snug text-stone-500 dark:text-neutral-400">
          {card.notes}
        </p>
      )}
      {card.status === 'blocked' && card.blocker && (
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
    .replace('{fetched}', String(outcome.fetched));
}

function TaskSourceControls({ disabled, compact }: { disabled: boolean; compact: boolean }) {
  const { t } = useT();
  const navigate = useNavigate();
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

  return (
    <section className="mb-3 rounded-lg border border-stone-200 bg-white p-3 dark:border-neutral-800 dark:bg-neutral-900">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="min-w-0">
          <h5 className="text-xs font-semibold text-stone-800 dark:text-neutral-100">
            {t('conversations.taskKanban.sources.title')}
          </h5>
          {!compact && status && (
            <p className="text-[11px] text-stone-500 dark:text-neutral-400">
              {status.enabled
                ? t('conversations.taskKanban.sources.statusEnabled')
                : t('settings.taskSources.disabledBanner')}
            </p>
          )}
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => navigate('/settings/task-sources')}
            className="text-[11px] font-medium text-ocean-600 hover:text-ocean-700 dark:text-ocean-300 dark:hover:text-ocean-200">
            {t('conversations.taskKanban.sources.manage')}
          </button>
          <button
            type="button"
            aria-label={t('settings.taskSources.refresh')}
            disabled={loading}
            onClick={() => void load()}
            className="flex h-7 w-7 items-center justify-center rounded-md border border-stone-200 text-stone-500 hover:bg-stone-50 disabled:opacity-40 dark:border-neutral-800 dark:text-neutral-300 dark:hover:bg-neutral-800">
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
        <p className="mt-2 text-[11px] text-stone-400 dark:text-neutral-500">
          {t('common.loading')}
        </p>
      ) : sources.length === 0 ? (
        <p className="mt-2 text-[11px] text-stone-400 dark:text-neutral-500">
          {t('settings.taskSources.empty')}
        </p>
      ) : (
        <ul className="mt-3 grid gap-2 sm:grid-cols-2">
          {sources.map(source => (
            <li
              key={source.id}
              className="min-w-0 rounded-lg border border-stone-200 px-2.5 py-2 dark:border-neutral-800">
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <p className="truncate text-xs font-medium text-stone-800 dark:text-neutral-100">
                    {source.name || providerLabel(source.provider, t)}
                  </p>
                  <p className="truncate text-[11px] text-stone-500 dark:text-neutral-400">
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
                      : 'bg-stone-100 text-stone-500 dark:bg-neutral-800 dark:text-neutral-400'
                  }`}>
                  {source.enabled
                    ? t('settings.taskSources.statusEnabled')
                    : t('settings.taskSources.statusDisabled')}
                </span>
              </div>
              <div className="mt-2 flex flex-wrap gap-1.5">
                <button
                  type="button"
                  disabled={disabled || busyKey === `fetch:${source.id}`}
                  onClick={() => void fetchSource(source)}
                  className="inline-flex items-center gap-1 rounded-md border border-stone-200 px-2 py-1 text-[11px] font-medium text-stone-600 hover:bg-stone-50 disabled:opacity-40 dark:border-neutral-800 dark:text-neutral-300 dark:hover:bg-neutral-800">
                  <LuRefreshCw className="h-3 w-3" />
                  {busyKey === `fetch:${source.id}`
                    ? t('settings.taskSources.fetching')
                    : t('settings.taskSources.fetchNow')}
                </button>
                <button
                  type="button"
                  disabled={disabled || busyKey === `toggle:${source.id}`}
                  onClick={() => void toggleSource(source)}
                  className="rounded-md border border-stone-200 px-2 py-1 text-[11px] font-medium text-stone-600 hover:bg-stone-50 disabled:opacity-40 dark:border-neutral-800 dark:text-neutral-300 dark:hover:bg-neutral-800">
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
      <section className="max-h-full w-full max-w-xl overflow-y-auto rounded-lg border border-stone-200 bg-white p-4 shadow-xl dark:border-neutral-800 dark:bg-neutral-900">
        <div className="mb-3 flex items-start justify-between gap-3">
          <div className="min-w-0">
            <p className="text-[11px] font-semibold uppercase text-stone-400 dark:text-neutral-500">
              {t('conversations.taskKanban.briefTitle')}
            </p>
            <h3 className="break-words text-base font-semibold text-stone-900 dark:text-neutral-50">
              {card.title}
            </h3>
          </div>
          <button
            type="button"
            aria-label={t('conversations.taskKanban.closeBrief')}
            onClick={onClose}
            className="flex h-7 w-7 flex-none items-center justify-center rounded-md text-stone-500 hover:bg-stone-100 hover:text-stone-800 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100">
            <LuX className="h-4 w-4" />
          </button>
        </div>

        {source && <SourceBrief source={source} />}

        {editable ? (
          <div className="space-y-3 text-sm">
            <label className="block">
              <span className="mb-1 block text-xs font-semibold text-stone-500 dark:text-neutral-400">
                {t('conversations.taskKanban.field.title')}
              </span>
              <input
                value={title}
                onChange={e => setTitle(e.target.value)}
                className="w-full rounded-md border border-stone-200 bg-white px-2 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-950 dark:text-neutral-50"
              />
            </label>
            <div className="grid gap-3 sm:grid-cols-3">
              <label className="block">
                <span className="mb-1 block text-xs font-semibold text-stone-500 dark:text-neutral-400">
                  {t('conversations.taskKanban.field.status')}
                </span>
                <select
                  value={status}
                  onChange={e => setStatus(e.target.value as TaskBoardCardStatus)}
                  className="w-full rounded-md border border-stone-200 bg-white px-2 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-950 dark:text-neutral-50">
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
                <span className="mb-1 block text-xs font-semibold text-stone-500 dark:text-neutral-400">
                  {t('conversations.taskKanban.field.approval')}
                </span>
                <select
                  value={approvalMode}
                  onChange={e => setApprovalMode(e.target.value)}
                  className="w-full rounded-md border border-stone-200 bg-white px-2 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-950 dark:text-neutral-50">
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
                <button
                  type="button"
                  onClick={handleDelete}
                  className="rounded-md border border-coral-200 px-3 py-1.5 text-xs font-medium text-coral-600 hover:bg-coral-50 dark:border-coral-500/30 dark:text-coral-300 dark:hover:bg-coral-500/10">
                  {t('conversations.taskKanban.deleteCard')}
                </button>
              ) : (
                <span />
              )}
              <div className="flex gap-2">
                <button
                  type="button"
                  onClick={onClose}
                  className="rounded-md border border-stone-200 px-3 py-1.5 text-xs font-medium text-stone-600 hover:bg-stone-50 dark:border-neutral-700 dark:text-neutral-300 dark:hover:bg-neutral-800">
                  {t('common.cancel')}
                </button>
                <button
                  type="button"
                  onClick={save}
                  disabled={!title.trim()}
                  className="rounded-md bg-ocean-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-ocean-700 disabled:opacity-50">
                  {t('conversations.taskKanban.saveChanges')}
                </button>
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
                <button
                  type="button"
                  onClick={handleDelete}
                  className="rounded-md border border-coral-200 px-3 py-1.5 text-xs font-medium text-coral-600 hover:bg-coral-50 dark:border-coral-500/30 dark:text-coral-300 dark:hover:bg-coral-500/10">
                  {t('conversations.taskKanban.deleteCard')}
                </button>
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
      <dd
        className={`mt-0.5 break-words text-xs text-stone-800 dark:text-neutral-100 ${
          mono ? 'font-mono' : ''
        }`}>
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
      <span className="mb-1 block text-xs font-semibold text-stone-500 dark:text-neutral-400">
        {label}
      </span>
      <input
        value={value}
        onChange={e => onChange(e.target.value)}
        className="w-full rounded-md border border-stone-200 bg-white px-2 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-950 dark:text-neutral-50"
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
      <span className="mb-1 block text-xs font-semibold text-stone-500 dark:text-neutral-400">
        {label}
      </span>
      <textarea
        value={value}
        onChange={e => onChange(e.target.value)}
        rows={3}
        className="w-full resize-y rounded-md border border-stone-200 bg-white px-2 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-950 dark:text-neutral-50"
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
      <h4 className="mb-1 text-xs font-semibold text-stone-500 dark:text-neutral-400">{label}</h4>
      <p
        className={`break-words text-sm ${
          mono ? 'font-mono' : ''
        } ${tone === 'danger' ? 'text-coral-600' : 'text-stone-800 dark:text-neutral-100'}`}>
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
      <h4 className="mb-1 text-xs font-semibold text-stone-500 dark:text-neutral-400">{label}</h4>
      <List
        className={`space-y-1 ${
          ordered ? 'list-decimal' : 'list-disc'
        } list-inside text-sm text-stone-800 dark:text-neutral-100 ${mono ? 'font-mono' : ''}`}>
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
