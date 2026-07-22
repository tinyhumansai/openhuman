/**
 * FlowRunsDrawer (issue B5a.1)
 * ----------------------------
 *
 * Right-side drawer listing a flow's run history, opened from the
 * "View runs" action on {@link FlowListRow}. Drawer chrome mirrors
 * `FlowRunInspectorDrawer`/`SubagentDrawer` (fixed overlay + backdrop-click-
 * to-close + Escape-to-close via `useEscapeKey`) so it renders as a fixed
 * overlay regardless of where the parent mounts it.
 *
 * Data loads via `listFlowRuns` on open, then stays live via
 * {@link useFlowRunsLiveRefresh} while any run in the list is still active —
 * so a run stuck on "Running" here updates without the user having to close
 * and reopen the drawer. The run inspector separately polls a single run's
 * live status via `useFlowRunPoller`; that's unrelated to this list refresh.
 *
 * Clicking a run sets `selectedRunId` and renders the existing
 * `FlowRunInspectorDrawer` stacked on top: both are `fixed inset-0 z-50`
 * overlays, and the inspector is rendered *after* this drawer's own overlay
 * in the JSX, so it paints above it (same stacking context, later DOM wins)
 * and its backdrop naturally intercepts clicks meant for the runs list.
 * Closing the inspector clears `selectedRunId` and returns to the run list;
 * closing this drawer (✕ / backdrop / Escape) calls `onClose`. While the
 * inspector is open, this drawer's own Escape handler is disabled so a
 * single Escape press closes only the topmost overlay (the inspector) first.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useEscapeKey } from '../../hooks/useEscapeKey';
import { useFlowRunsLiveRefresh } from '../../hooks/useFlowRunsLiveRefresh';
import { useFlowRunStarted } from '../../hooks/useFlowRunStarted';
import {
  resolveDisplayStatus,
  useRunsPendingApprovalSet,
} from '../../hooks/useRunsPendingApprovalSet';
import { useT } from '../../lib/i18n/I18nContext';
import { type FlowRun, listFlowRuns } from '../../services/api/flowsApi';
import {
  FLOW_RUN_STATUS_ACCENT,
  FLOW_RUN_STATUS_DOT,
  FLOW_RUN_STATUS_KEY,
  type FlowRepairRequest,
  FlowRunInspectorDrawer,
} from './FlowRunInspectorDrawer';

const log = debug('flows:runs-drawer');

function formatTimestamp(value: string | null | undefined): string | null {
  if (!value) return null;
  const parsed = Date.parse(value);
  if (!Number.isFinite(parsed)) return null;
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  }).format(new Date(parsed));
}

interface Props {
  /** Flow to list runs for. Renders `null` (nothing) when absent. */
  flowId: string | null;
  /** Flow name for the drawer title, when known. */
  flowName?: string;
  onClose: () => void;
  /** "Fix with agent" (Phase 5c) — forwarded to the inspector for failed runs. */
  onFixWithAgent?: (request: FlowRepairRequest) => void;
}

/**
 * Renders `null` when `flowId` is `null` so the parent can mount this
 * unconditionally and just flip `flowId` (same convention as
 * `FlowRunInspectorDrawer`/`SubagentDrawer`).
 */
export function FlowRunsDrawer({ flowId, flowName, onClose, onFixWithAgent }: Props) {
  const { t } = useT();
  const [runs, setRuns] = useState<FlowRun[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  // Tracks the flowId this drawer instance is *currently* showing, so an
  // in-flight `refetch()` started for a previous flow (see below) can detect
  // it's stale once the drawer flips to a new flowId and bail instead of
  // clobbering the new flow's already-loaded runs.
  const currentFlowIdRef = useRef(flowId);
  // Per-request generation counter, shared by the initial load effect and
  // `refetch` below: a request started before a run-started event (or before
  // a newer refetch) can resolve AFTER it and, without this guard, clobber a
  // fresh "Running" row with stale data — even for the SAME flowId, where the
  // `currentFlowIdRef` check alone can't tell requests apart. Only the
  // most-recently-issued request for the current flow may apply its result.
  const requestGenRef = useRef(0);

  useEffect(() => {
    currentFlowIdRef.current = flowId;

    // Reset for the new target so a previous flow's runs/error can't linger
    // under a different flowId while the new fetch is in flight.
    setSelectedRunId(null);
    setError(null);

    if (!flowId) {
      setRuns([]);
      setLoading(false);
      return;
    }

    let cancelled = false;
    const requestGen = ++requestGenRef.current;
    setLoading(true);
    log('loading runs: flowId=%s', flowId);
    listFlowRuns(flowId)
      .then(result => {
        if (cancelled || requestGen !== requestGenRef.current) return;
        setRuns(result);
        log('loaded runs: flowId=%s count=%d', flowId, result.length);
      })
      .catch(err => {
        if (cancelled || requestGen !== requestGenRef.current) return;
        const msg = err instanceof Error ? err.message : String(err);
        log('load failed: flowId=%s err=%s', flowId, msg);
        setError(msg);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [flowId]);

  // Background refresh for the live-update hook below — deliberately doesn't
  // touch `loading`/`error` so a poll tick or progress event never flashes
  // the loading state or clobbers a real load error with a transient one.
  // Guards against a stale response two ways: if the drawer flips from flow A
  // to flow B while an A refetch is still in flight, the late A response must
  // not overwrite B's already-loaded runs (`currentFlowIdRef`); and if two
  // requests for the SAME flow race (e.g. the initial load and an
  // event-driven refetch, or two refetches back to back), only the response
  // to the most-recently-issued one may apply (`requestGenRef`).
  const refetch = useCallback(() => {
    if (!flowId) return;
    const requestFlowId = flowId;
    const requestGen = ++requestGenRef.current;
    listFlowRuns(requestFlowId)
      .then(result => {
        if (currentFlowIdRef.current !== requestFlowId || requestGen !== requestGenRef.current) {
          return;
        }
        setRuns(result);
        log('refetched runs: flowId=%s count=%d', requestFlowId, result.length);
      })
      .catch(err => {
        if (currentFlowIdRef.current !== requestFlowId || requestGen !== requestGenRef.current) {
          return;
        }
        const msg = err instanceof Error ? err.message : String(err);
        log('refetch failed: flowId=%s err=%s', requestFlowId, msg);
      });
  }, [flowId]);

  useFlowRunsLiveRefresh(runs, refetch);
  // Unconditional (unlike useFlowRunsLiveRefresh, which is gated on an
  // already-active run) — fills the empty-list gap ("No runs yet") that hook
  // can't reach, so the very first run shows up as "Running" instantly
  // instead of waiting for a manual refresh (issue B35).
  useFlowRunStarted(() => void refetch(), flowId);
  const pendingRunIds = useRunsPendingApprovalSet(runs);

  useEscapeKey(
    () => {
      log('escape: closing flowId=%s', flowId);
      onClose();
    },
    flowId !== null && selectedRunId === null
  );

  if (!flowId) return null;

  const title = flowName
    ? t('flows.runs.title').replace('{name}', flowName)
    : t('flows.runs.titleFallback');

  return (
    <>
      <div className="fixed inset-0 z-50 flex justify-end" data-testid="flow-runs-drawer">
        {/* Backdrop */}
        <button
          type="button"
          aria-label={t('conversations.subagent.close')}
          data-testid="flow-runs-backdrop"
          className="absolute inset-0 bg-stone-900/30 dark:bg-black/50"
          onClick={onClose}
        />
        <aside className="relative flex h-full w-full max-w-md flex-col bg-surface shadow-xl">
          {/* Header */}
          <header className="flex items-center gap-2.5 border-b border-line px-4 py-3">
            <span className="min-w-0 flex-1 truncate font-semibold text-content">{title}</span>
            <button
              type="button"
              data-testid="flow-runs-close"
              onClick={onClose}
              aria-label={t('conversations.subagent.close')}
              className="shrink-0 rounded-full p-1.5 text-content-faint hover:bg-surface-hover hover:text-content-secondary">
              ✕
            </button>
          </header>

          <div className="flex-1 overflow-y-auto px-4 py-4">
            {loading && (
              <div
                className="flex items-center gap-2 py-8 text-content-faint"
                data-testid="flow-runs-loading">
                <div className="h-4 w-4 animate-spin rounded-full border-2 border-ocean-500 border-t-transparent" />
                <span className="text-sm">{t('flows.runs.loading')}</span>
              </div>
            )}

            {error && (
              <div
                role="alert"
                data-testid="flow-runs-error"
                className="rounded-xl border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
                {t('flows.runs.loadError')}: {error}
              </div>
            )}

            {!loading && !error && runs.length === 0 && (
              <p
                className="py-8 text-center text-xs italic text-content-faint"
                data-testid="flow-runs-empty">
                {t('flows.runs.empty')}
              </p>
            )}

            {!loading && !error && runs.length > 0 && (
              <ul className="space-y-2" data-testid="flow-runs-list">
                {runs.map(run => {
                  const startedAt = formatTimestamp(run.started_at);
                  const displayStatus = resolveDisplayStatus(run, pendingRunIds);
                  return (
                    <li key={run.id}>
                      <button
                        type="button"
                        data-testid={`flow-run-row-${run.id}`}
                        onClick={() => setSelectedRunId(run.id)}
                        className="flex w-full items-center gap-2 rounded-lg border border-line bg-surface-muted px-3 py-2 text-left text-xs hover:bg-surface-hover">
                        <span
                          data-testid={`flow-run-row-dot-${run.id}`}
                          className={`h-2 w-2 shrink-0 rounded-full ${FLOW_RUN_STATUS_DOT[displayStatus]}`}
                          aria-hidden
                        />
                        <span
                          className={`inline-flex shrink-0 items-center rounded-full border px-2 py-0.5 font-medium ${FLOW_RUN_STATUS_ACCENT[displayStatus]}`}>
                          {t(FLOW_RUN_STATUS_KEY[displayStatus])}
                        </span>
                        {startedAt && (
                          <span className="truncate text-content-muted">{startedAt}</span>
                        )}
                        <span className="ml-auto truncate font-mono text-[10px] text-content-faint">
                          {run.id.slice(0, 8)}
                        </span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
        </aside>
      </div>

      {selectedRunId && (
        <FlowRunInspectorDrawer
          runId={selectedRunId}
          onClose={() => setSelectedRunId(null)}
          onFixWithAgent={onFixWithAgent}
        />
      )}
    </>
  );
}

export default FlowRunsDrawer;
