/**
 * FlowRunsSidebar — the workflow's recent runs, projected into the root shell's
 * dynamic left sidebar while a flow is open on the canvas (`/flows/:id`). A
 * compact, scannable run history (status dot + status + relative time); clicking
 * a run opens the full {@link FlowRunInspectorDrawer} (which polls its live
 * status). Fetches via `useFlowRunsQuery`, with a manual refresh button plus
 * {@link useFlowRunsLiveRefresh} keeping the list itself live while any run
 * shown here is still active (no manual refresh/navigate-away required).
 *
 * Rendered by `FlowCanvasPage` inside a `SidebarContent` portal, so it only
 * appears for a persisted flow (a draft has no runs yet).
 */
import createDebug from 'debug';
import { useCallback, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useFlowRunFinished } from '../../hooks/useFlowRunFinished';
import { useFlowRunsLiveRefresh } from '../../hooks/useFlowRunsLiveRefresh';
import { useFlowRunsQuery } from '../../hooks/useFlowRunsQuery';
import { useFlowRunStarted } from '../../hooks/useFlowRunStarted';
import {
  resolveDisplayStatus,
  useRunsPendingApprovalSet,
} from '../../hooks/useRunsPendingApprovalSet';
import { useT } from '../../lib/i18n/I18nContext';
import { CenteredLoadingState, ErrorBanner } from '../ui/LoadingState';
import { type FlowRepairRequest, FlowRunInspectorDrawer } from './FlowRunInspectorDrawer';
import { FlowRunStatus, flowRunStatusLabel } from './FlowRunStatus';

/** Matches `useT()`'s `t` signature. */
type TFn = (key: string, fallback?: string) => string;

function relativeTime(iso: string, t: TFn): string {
  const ms = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(ms / 60000);
  if (mins < 1) return t('flows.list.justNow');
  if (mins < 60) return t('flows.list.minutesAgo').replace('{count}', String(mins));
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return t('flows.list.hoursAgo').replace('{count}', String(hrs));
  const days = Math.floor(hrs / 24);
  return t('flows.list.daysAgo').replace('{count}', String(days));
}

const log = createDebug('app:flows:runs-sidebar');

interface FlowRunsSidebarProps {
  flowId: string;
}

export default function FlowRunsSidebar({ flowId }: FlowRunsSidebarProps) {
  const { t } = useT();
  const navigate = useNavigate();
  const { runs, loading, error, refresh, refreshSilently } = useFlowRunsQuery({
    scope: { kind: 'flow', flowId },
  });
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);

  // "Fix with agent" (issue B22) — this sidebar is only ever mounted while
  // already on the failed run's own `/flows/:id` canvas (`FlowCanvasPage`
  // projects it into the shell sidebar), so re-navigating to the SAME route
  // with a fresh `copilotRepair` state is enough to open the canvas copilot
  // preloaded with the failure — same mechanism `FlowsPage`'s run-history
  // drawer uses to reach this page from elsewhere. `replace: true` avoids
  // stacking a new history entry per click on top of the page the user is
  // already viewing.
  const handleFixWithAgent = useCallback(
    (request: FlowRepairRequest) => {
      log('fix with agent: flow=%s run=%s', request.flowId, request.runId);
      setSelectedRunId(null);
      navigate(`/flows/${request.flowId}`, {
        replace: true,
        state: {
          copilotRepair: {
            runId: request.runId,
            error: request.error,
            failingNodeIds: request.failingNodeIds,
          },
        },
      });
    },
    [navigate]
  );

  const handleRunStarted = useCallback(() => {
    log('run-started: refetch flow=%s', flowId);
    void refreshSilently();
  }, [flowId, refreshSilently]);
  const handleRunFinished = useCallback(() => {
    log('run-finished: refetch flow=%s', flowId);
    void refreshSilently();
  }, [flowId, refreshSilently]);

  useFlowRunsLiveRefresh(runs, refreshSilently);
  // Unconditional (unlike useFlowRunsLiveRefresh, which is gated on an
  // already-active run) — fills the empty-list gap ("No runs yet") that
  // hook can't reach, so the very first run shows up as "Running" instantly
  // instead of waiting for a manual refresh (issue B35).
  useFlowRunStarted(handleRunStarted, flowId);
  // Terminal companion to the above (issue B35 follow-up) — flips a run to
  // Completed/Failed the instant it settles instead of waiting on
  // `useFlowRunsLiveRefresh`'s debounced/backstop refetch to notice.
  useFlowRunFinished(handleRunFinished, flowId);
  const pendingRunIds = useRunsPendingApprovalSet(runs);

  return (
    <div className="flex h-full flex-col" data-testid="flow-runs-sidebar">
      <div className="flex flex-shrink-0 items-center justify-between gap-2 px-3 py-2">
        <span className="text-[11px] font-semibold uppercase tracking-wide text-content-faint">
          {t('flows.runs.sidebarTitle')}
        </span>
        <button
          type="button"
          onClick={() => void refresh()}
          disabled={loading}
          data-testid="flow-runs-sidebar-refresh"
          aria-label={t('flows.runs.refresh')}
          title={t('flows.runs.refresh')}
          className="rounded-md p-1 text-content-faint transition-colors hover:bg-surface-hover hover:text-content-secondary disabled:opacity-50">
          <svg
            className="h-3.5 w-3.5"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
            aria-hidden="true">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M4 4v5h5M20 20v-5h-5M4 9a8 8 0 0114-3m2 8a8 8 0 01-14 3"
            />
          </svg>
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
        {loading && runs.length === 0 && <CenteredLoadingState label={t('flows.runs.loading')} />}

        {error && (
          <div className="px-1">
            <ErrorBanner message={error} />
          </div>
        )}

        {!loading && !error && runs.length === 0 && (
          <p
            className="px-2 py-6 text-center text-xs text-content-faint"
            data-testid="flow-runs-sidebar-empty">
            {t('flows.runs.empty')}
          </p>
        )}

        <ul className="space-y-1">
          {runs.map(run => {
            const displayStatus = resolveDisplayStatus(run, pendingRunIds);
            return (
              <li key={run.id}>
                <button
                  type="button"
                  data-testid={`flow-runs-sidebar-run-${run.id}`}
                  onClick={() => setSelectedRunId(run.id)}
                  className={`flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-surface-hover ${
                    selectedRunId === run.id ? 'bg-surface-hover' : ''
                  }`}>
                  <FlowRunStatus
                    status={displayStatus}
                    label={flowRunStatusLabel(displayStatus, t)}
                    presentation="dot"
                  />
                  <span className="min-w-0 flex-1">
                    <FlowRunStatus
                      status={displayStatus}
                      label={flowRunStatusLabel(displayStatus, t)}
                      className="!px-1.5 text-[10px]"
                    />
                    <span className="mt-0.5 block truncate text-[11px] text-content-faint">
                      {relativeTime(run.started_at, t)}
                    </span>
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      </div>

      <FlowRunInspectorDrawer
        runId={selectedRunId}
        onClose={() => setSelectedRunId(null)}
        onFixWithAgent={handleFixWithAgent}
      />
    </div>
  );
}
