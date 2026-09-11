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
import { Button, CenteredLoadingState, EmptyState, ErrorBanner } from '../ui';
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
    // Laid out as one of the shell's sidebar regions, not as a bespoke panel:
    // the heading and the rows below reuse `TwoPaneNav`'s exact specs (10px
    // uppercase group label at `pt-0`; `h-auto w-full justify-start rounded-md
    // px-2.5 py-1.5 text-[14px]` rows, primary-filled when active), so this
    // list and the nav group above it in the same column read as one sidebar
    // rather than two components that happen to be stacked.
    <div className="flex h-full flex-col" data-testid="flow-runs-sidebar">
      {/* `px-3` on the row with `px-2` on the label puts the heading on the same
          left edge as the row text below it (`px-3` list + `px-2.5` button),
          which is exactly how `TwoPaneNav` insets its own group headings. The
          row used to be `px-2` against a `px-3` list, so the heading sat ~14px
          left of everything under it and the refresh button hugged the pane
          edge. */}
      <div className="flex shrink-0 items-center justify-between gap-2 px-3 pb-1">
        <span className="px-2 text-[10px] font-semibold uppercase tracking-wider text-content-muted">
          {t('flows.runs.sidebarTitle')}
        </span>
        <Button
          type="button"
          variant="tertiary"
          size="xs"
          iconOnly
          onClick={() => void refresh()}
          disabled={loading}
          data-testid="flow-runs-sidebar-refresh"
          aria-label={t('flows.runs.refresh')}
          title={t('flows.runs.refresh')}>
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
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-2">
        {loading && runs.length === 0 && <CenteredLoadingState label={t('flows.runs.loading')} />}

        {error && (
          <div className="px-1">
            <ErrorBanner message={error} />
          </div>
        )}

        {!loading && !error && runs.length === 0 && (
          <EmptyState
            className="px-2"
            label={t('flows.runs.empty')}
            data-testid="flow-runs-sidebar-empty"
          />
        )}

        <ul>
          {runs.map(run => {
            const displayStatus = resolveDisplayStatus(run, pendingRunIds);
            const statusLabel = flowRunStatusLabel(displayStatus, t);
            const active = selectedRunId === run.id;
            return (
              <li key={run.id}>
                <Button
                  type="button"
                  variant="tertiary"
                  aria-current={active ? 'page' : undefined}
                  data-testid={`flow-runs-sidebar-run-${run.id}`}
                  onClick={() => setSelectedRunId(run.id)}
                  // `TwoPaneNav`'s row spec, tightened to a single line.
                  //
                  // The status used to be painted TWICE per row — a coloured
                  // dot AND the same status again as an accented badge, with
                  // the time stacked underneath — so a list of runs was three
                  // visual weights deep and two lines tall. The dot carries
                  // the colour; status and time share one line, which also
                  // fixes the dot's alignment: against a two-line stack it
                  // centred on the block rather than sitting on the text.
                  className={`h-auto w-full justify-start gap-2 rounded-md px-2.5 py-1 text-left text-[13px] ${
                    active
                      ? 'bg-primary-500 font-semibold text-content-inverted hover:bg-primary-500'
                      : 'font-normal text-content-muted hover:bg-surface/40 hover:text-content-secondary'
                  }`}>
                  <FlowRunStatus status={displayStatus} label={statusLabel} presentation="dot" />
                  <span className="min-w-0 flex-1 truncate">{statusLabel}</span>
                  <span
                    className={`shrink-0 text-[11px] font-normal tabular-nums ${
                      active ? 'text-content-inverted/70' : 'text-content-faint'
                    }`}>
                    {relativeTime(run.started_at, t)}
                  </span>
                </Button>
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
