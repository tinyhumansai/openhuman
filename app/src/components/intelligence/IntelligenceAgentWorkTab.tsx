/**
 * IntelligenceAgentWorkTab — the Background Agent Command Center.
 *
 * Reads `openhuman.agent_work_list` (via {@link agentWorkApi}) once on mount
 * and renders every tracked background agent run, grouped into five lifecycle
 * buckets in a fixed display order: needs_input → working → completed →
 * failed → stopped. The handler always returns all five groups, so the order
 * here is authoritative and never needs sorting.
 *
 * This tab is read-only and holds no Redux slice — it owns its own
 * {data, loading, error} state, mirroring {@link IntelligenceTasksTab}'s
 * mount pattern (mountedRef + a 0ms `setTimeout` so the first paint shows the
 * loading state before the RPC resolves). Each row offers jumps to the parent
 * / worker thread via the same /chat/:threadId path the
 * Tasks tab uses for "View session".
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type AgentWorkAction,
  agentWorkApi,
  type AgentWorkBucket,
  type AgentWorkResponse,
  type AgentWorkRow,
} from '../../services/api/agentWorkApi';
import { useAppDispatch } from '../../store/hooks';
import { loadThreadMessages, loadThreads, setSelectedThread } from '../../store/threadSlice';
import { chatThreadPath } from '../../utils/chatRoutes';

const log = debug('intelligence:agent-work');

/** Fixed display order of buckets — matches the handler's group order. */
const BUCKET_ORDER: AgentWorkBucket[] = [
  'needs_input',
  'working',
  'completed',
  'failed',
  'stopped',
];

/** Per-bucket accent classes (semantic palette from tailwind.config.js). */
const BUCKET_ACCENT: Record<AgentWorkBucket, string> = {
  needs_input:
    'border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-300',
  working:
    'border-ocean-200 bg-ocean-50 text-ocean-700 dark:border-ocean-500/30 dark:bg-ocean-500/10 dark:text-ocean-300',
  completed:
    'border-sage-200 bg-sage-50 text-sage-700 dark:border-sage-500/30 dark:bg-sage-500/10 dark:text-sage-300',
  failed:
    'border-coral-200 bg-coral-50 text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300',
  stopped: 'border-line bg-surface-muted text-content-secondary',
};

/** i18n key for each bucket's localized label. */
const BUCKET_LABEL_KEY: Record<AgentWorkBucket, string> = {
  needs_input: 'intelligence.agentWork.bucket.needsInput',
  working: 'intelligence.agentWork.bucket.working',
  completed: 'intelligence.agentWork.bucket.completed',
  failed: 'intelligence.agentWork.bucket.failed',
  stopped: 'intelligence.agentWork.bucket.stopped',
};

/** i18n key for each granular run status (mirrors Rust `AgentRunStatus`). */
const STATUS_LABEL_KEY: Record<string, string> = {
  pending: 'intelligence.agentWork.status.pending',
  running: 'intelligence.agentWork.status.running',
  awaiting_user: 'intelligence.agentWork.status.awaitingUser',
  paused: 'intelligence.agentWork.status.paused',
  completed: 'intelligence.agentWork.status.completed',
  failed: 'intelligence.agentWork.status.failed',
  cancelled: 'intelligence.agentWork.status.cancelled',
  interrupted: 'intelligence.agentWork.status.interrupted',
};

/** i18n key for each run kind (mirrors Rust `AgentRunKind`). */
const KIND_LABEL_KEY: Record<string, string> = {
  subagent: 'intelligence.agentWork.kind.subagent',
  worker_thread: 'intelligence.agentWork.kind.workerThread',
  background_agent: 'intelligence.agentWork.kind.backgroundAgent',
  team_member: 'intelligence.agentWork.kind.teamMember',
  workflow_child: 'intelligence.agentWork.kind.workflowChild',
};

/** Format an elapsed millisecond span as a compact "1h 23m" / "45s" string. */
export function formatElapsed(ms: number | undefined): string {
  if (ms === undefined || !Number.isFinite(ms) || ms < 0) return '—';
  const totalSeconds = Math.floor(ms / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${seconds}s`;
  return `${seconds}s`;
}

/** Human-readable token count: 1234 → "1.2K", 2_500_000 → "2.5M". */
export function formatTokens(value: number): string {
  if (!Number.isFinite(value) || value < 0) return '0';
  if (value < 1000) return String(value);
  if (value < 1_000_000) return `${(value / 1000).toFixed(1)}K`;
  return `${(value / 1_000_000).toFixed(1)}M`;
}

/** Format a USD cost with two decimals, e.g. 0.0123 → "$0.01". */
export function formatCost(value: number): string {
  if (!Number.isFinite(value) || value < 0) return '$0.00';
  return `$${value.toFixed(2)}`;
}

export default function IntelligenceAgentWorkTab() {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const navigate = useNavigate();

  const [data, setData] = useState<AgentWorkResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const fetchWork = useCallback(async () => {
    log('fetchWork: entry');
    setError(null);
    try {
      const response = await agentWorkApi.list();
      if (mountedRef.current) {
        setData(response);
        log('fetchWork: done total=%d', response.total);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('fetchWork: error %s', msg);
      if (mountedRef.current) setError(msg);
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    const handle = window.setTimeout(() => {
      void fetchWork();
    }, 0);
    return () => {
      window.clearTimeout(handle);
      mountedRef.current = false;
    };
  }, [fetchWork]);

  // Open a thread in /chat — mirrors IntelligenceTasksTab's "View session"
  // path so the chat surface lands on the requested thread instead of just
  // the Conversations list. Navigation only; the thread is not marked active.
  const openThread = useCallback(
    (threadId: string) => {
      log('openThread threadId=%s', threadId);
      dispatch(setSelectedThread(threadId));
      void dispatch(loadThreads());
      void dispatch(loadThreadMessages(threadId));
      navigate(chatThreadPath(threadId));
    },
    [dispatch, navigate]
  );

  if (loading) {
    return (
      <div className="flex items-center justify-center py-10 text-content-faint">
        <div className="mr-2 h-4 w-4 animate-spin rounded-full border-2 border-ocean-500 border-t-transparent" />
        <span className="text-sm">{t('intelligence.agentWork.loading')}</span>
      </div>
    );
  }

  if (error) {
    return (
      <div className="rounded-xl border border-coral-200 bg-coral-50 px-4 py-3 text-sm text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
        {t('intelligence.agentWork.failedToLoad')}: {error}
      </div>
    );
  }

  if (!data || data.total === 0) {
    return (
      <div className="space-y-4">
        <p className="text-xs text-content-faint">{t('intelligence.agentWork.subtitle')}</p>
        <div className="rounded-xl border border-dashed border-line py-10 text-center text-sm text-content-faint">
          {t('intelligence.agentWork.empty')}
        </div>
      </div>
    );
  }

  const groupByBucket = new Map(data.groups.map(group => [group.bucket, group]));

  return (
    <div className="space-y-6">
      <p className="text-xs text-content-faint">{t('intelligence.agentWork.subtitle')}</p>

      {BUCKET_ORDER.map(bucket => {
        const group = groupByBucket.get(bucket);
        const rows = group?.rows ?? [];
        if (rows.length === 0) return null;
        return (
          <section key={bucket} className="space-y-2">
            <div className="flex items-center gap-2">
              <span
                className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs font-medium ${BUCKET_ACCENT[bucket]}`}>
                {bucket === 'working' && (
                  <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-ocean-500" />
                )}
                {t(BUCKET_LABEL_KEY[bucket])}
              </span>
              <span className="text-xs text-content-faint">{group?.count ?? rows.length}</span>
            </div>

            <ul className="divide-y divide-line-subtle overflow-hidden rounded-xl border border-line bg-surface dark:divide-neutral-800">
              {rows.map(row => (
                <AgentWorkRowItem
                  key={row.runId}
                  row={row}
                  onOpenThread={openThread}
                  onControlled={fetchWork}
                />
              ))}
            </ul>
          </section>
        );
      })}
    </div>
  );
}

/** Statuses where a run is still live and can be stopped. */
const NON_TERMINAL_STATUSES = new Set(['pending', 'running', 'awaiting_user', 'paused']);
/** Statuses a run can be retried (re-queued) from. */
const RETRYABLE_STATUSES = new Set(['failed', 'cancelled', 'interrupted']);

/** Shared button styling for the row's secondary actions. */
const ACTION_BTN =
  'rounded-md border border-line px-2 py-1 text-[11px] font-medium hover:bg-surface-hover disabled:cursor-not-allowed disabled:opacity-50';

interface AgentWorkRowItemProps {
  row: AgentWorkRow;
  onOpenThread: (threadId: string) => void;
  /** Called after a control verb succeeds so the parent can refetch. */
  onControlled: () => void;
}

function AgentWorkRowItem({ row, onOpenThread, onControlled }: AgentWorkRowItemProps) {
  const { t } = useT();
  const name = row.displayName || row.agentId || row.runId;
  const totalTokens = row.inputTokens + row.outputTokens;
  // Localize the backend enum values, falling back to the raw value for any
  // status/kind the UI doesn't yet have a key for.
  const statusLabel = STATUS_LABEL_KEY[row.status] ? t(STATUS_LABEL_KEY[row.status]) : row.status;
  const kindLabel = KIND_LABEL_KEY[row.kind] ? t(KIND_LABEL_KEY[row.kind]) : row.kind;

  // Which message-bearing composer is open (continue | follow_up), if any.
  const [composer, setComposer] = useState<'continue' | 'follow_up' | null>(null);
  const [message, setMessage] = useState('');
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const canStop = NON_TERMINAL_STATUSES.has(row.status);
  const canRetry = RETRYABLE_STATUSES.has(row.status);
  const canContinue = row.status === 'awaiting_user';

  const runControl = useCallback(
    async (action: AgentWorkAction, msg?: string) => {
      log('runControl runId=%s action=%s', row.runId, action);
      setBusy(true);
      setActionError(null);
      try {
        await agentWorkApi.control({ runId: row.runId, action, message: msg });
        setComposer(null);
        setMessage('');
        onControlled();
      } catch (err) {
        const text = err instanceof Error ? err.message : String(err);
        log('runControl error %s', text);
        setActionError(text);
      } finally {
        setBusy(false);
      }
    },
    [row.runId, onControlled]
  );

  const openComposer = useCallback((mode: 'continue' | 'follow_up') => {
    setComposer(mode);
    setMessage('');
    setActionError(null);
  }, []);

  return (
    <li className="p-3">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-1.5">
            <span
              title={t('intelligence.agentWork.column.agent')}
              className="truncate text-sm font-medium text-content">
              {name}
            </span>
            <span
              title={t('intelligence.agentWork.column.status')}
              className="rounded-md border border-line px-1.5 py-0.5 text-[10px] font-medium text-content-muted">
              {statusLabel}
            </span>
            <span className="text-[10px] uppercase tracking-wide text-content-faint">
              {kindLabel}
            </span>
          </div>
          {row.summary && (
            <p className="line-clamp-2 break-words text-xs leading-snug text-content-muted">
              {row.summary}
            </p>
          )}
          {row.error && (
            <p className="line-clamp-2 break-words text-xs leading-snug text-coral-600 dark:text-coral-300">
              {row.error}
            </p>
          )}
        </div>

        <div className="flex flex-none items-center gap-3 text-xs text-content-muted">
          <span title={t('intelligence.agentWork.column.elapsed')}>
            {formatElapsed(row.elapsedMs)}
          </span>
          <span title={t('intelligence.agentWork.column.cost')}>{formatCost(row.costUsd)}</span>
          <span title={t('intelligence.agentWork.column.tokens')}>{formatTokens(totalTokens)}</span>
        </div>
      </div>

      <div className="mt-2 flex flex-wrap items-center gap-2">
        {/* Open / peek — jump to the parent or worker transcript. */}
        {row.parentThreadId && (
          <button
            type="button"
            onClick={() => onOpenThread(row.parentThreadId as string)}
            className={`${ACTION_BTN} text-ocean-600 dark:text-ocean-300`}>
            {t('intelligence.agentWork.openThread')}
          </button>
        )}
        {row.workerThreadId && (
          <button
            type="button"
            onClick={() => onOpenThread(row.workerThreadId as string)}
            className={`${ACTION_BTN} text-ocean-600 dark:text-ocean-300`}>
            {t('intelligence.agentWork.openWorker')}
          </button>
        )}

        {/* Control verbs — availability is driven by the run's status. */}
        {canContinue && (
          <button
            type="button"
            disabled={busy}
            onClick={() => openComposer('continue')}
            className={`${ACTION_BTN} text-sage-700 dark:text-sage-300`}>
            {t('intelligence.agentWork.action.continue')}
          </button>
        )}
        <button
          type="button"
          disabled={busy}
          onClick={() => openComposer('follow_up')}
          className={`${ACTION_BTN} text-content-secondary`}>
          {t('intelligence.agentWork.action.followUp')}
        </button>
        {canStop && (
          <button
            type="button"
            disabled={busy}
            onClick={() => void runControl('stop')}
            className={`${ACTION_BTN} text-coral-600 dark:text-coral-300`}>
            {t('intelligence.agentWork.action.stop')}
          </button>
        )}
        {canRetry && (
          <button
            type="button"
            disabled={busy}
            onClick={() => void runControl('retry')}
            className={`${ACTION_BTN} text-ocean-600 dark:text-ocean-300`}>
            {t('intelligence.agentWork.action.retry')}
          </button>
        )}
      </div>

      {composer && (
        <div className="mt-2 space-y-2">
          <textarea
            aria-label={t(
              composer === 'continue'
                ? 'intelligence.agentWork.action.continuePlaceholder'
                : 'intelligence.agentWork.action.followUpPlaceholder'
            )}
            value={message}
            onChange={e => setMessage(e.target.value)}
            disabled={busy}
            rows={2}
            placeholder={t(
              composer === 'continue'
                ? 'intelligence.agentWork.action.continuePlaceholder'
                : 'intelligence.agentWork.action.followUpPlaceholder'
            )}
            className="w-full resize-y rounded-md border border-line bg-surface px-2 py-1.5 text-xs text-content focus:border-ocean-400 focus:outline-none"
          />
          <div className="flex items-center gap-2">
            <button
              type="button"
              disabled={busy || message.trim().length === 0}
              onClick={() => void runControl(composer, message)}
              className="rounded-md bg-ocean-500 px-2.5 py-1 text-[11px] font-medium text-white hover:bg-ocean-600 disabled:cursor-not-allowed disabled:opacity-50">
              {t('intelligence.agentWork.action.send')}
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => setComposer(null)}
              className={`${ACTION_BTN} text-content-muted`}>
              {t('intelligence.agentWork.action.cancel')}
            </button>
          </div>
        </div>
      )}

      {actionError && (
        <p className="mt-2 text-[11px] text-coral-600 dark:text-coral-300">
          {t('intelligence.agentWork.action.failed')}: {actionError}
        </p>
      )}
    </li>
  );
}
