/**
 * WorkflowRunsPage — the aggregate "All runs" view: every workflow's runs across
 * the whole `flows` domain, newest first, backed by the `flows_list_all_runs`
 * core RPC. Each row links back to its workflow's canvas. Stays live via
 * {@link useFlowRunsLiveRefresh} while any listed run is still active, via a
 * lightweight silent refresh (re-fetches just the runs, not `listFlows()` too)
 * so a run doesn't sit on "Running" until the user reloads the page.
 */
import { useCallback, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { FlowRunStatus } from '../components/flows/FlowRunStatus';
import SettingsTabbedPage from '../components/settings/layout/SettingsTabbedPage';
import { CenteredLoadingState, ErrorBanner } from '../components/ui/LoadingState';
import { useFlowRunFinished } from '../hooks/useFlowRunFinished';
import { useFlowRunsLiveRefresh } from '../hooks/useFlowRunsLiveRefresh';
import { useFlowRunsQuery } from '../hooks/useFlowRunsQuery';
import { useFlowRunStarted } from '../hooks/useFlowRunStarted';
import {
  resolveDisplayStatus,
  useRunsPendingApprovalSet,
} from '../hooks/useRunsPendingApprovalSet';
import { useT } from '../lib/i18n/I18nContext';
import {
  type Flow,
  type FlowRunStatus as FlowRunStatusValue,
  listFlows,
} from '../services/api/flowsApi';

export default function WorkflowRunsPage() {
  const { t } = useT();
  const navigate = useNavigate();
  const { runs, loading, error, refreshSilently } = useFlowRunsQuery({ scope: { kind: 'all' } });
  const [flowNames, setFlowNames] = useState<Record<string, string>>({});
  const [flowNamesLoading, setFlowNamesLoading] = useState(true);
  const [flowNamesError, setFlowNamesError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setFlowNamesLoading(true);
    setFlowNamesError(null);
    listFlows()
      .then(flows => {
        if (cancelled) return;
        const names: Record<string, string> = {};
        flows.forEach((flow: Flow) => {
          names[flow.id] = flow.name;
        });
        setFlowNames(names);
      })
      .catch(nameError => {
        if (cancelled) return;
        setFlowNamesError(nameError instanceof Error ? nameError.message : String(nameError));
      })
      .finally(() => {
        if (!cancelled) setFlowNamesLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, []);

  const handleRunFinished = useCallback(() => void refreshSilently(), [refreshSilently]);
  const handleRunStarted = useCallback(() => void refreshSilently(), [refreshSilently]);
  useFlowRunsLiveRefresh(runs, refreshSilently);
  useFlowRunFinished(handleRunFinished);
  // Unconditional (unlike useFlowRunsLiveRefresh, which is gated on an
  // already-active run) — fills the empty-list gap ("No runs yet") that hook
  // can't reach, so the very first run across any flow shows up as "Running"
  // instantly instead of waiting for a manual refresh (issue B35). No
  // `flowId` filter — this is the flow-agnostic "all runs" page.
  useFlowRunStarted(handleRunStarted);
  const pendingRunIds = useRunsPendingApprovalSet(runs);
  const pageLoading = loading || flowNamesLoading;
  const pageError = error ?? flowNamesError;

  const statusLabel = (status: FlowRunStatusValue) =>
    t(`flows.allRuns.status.${status}`, status.replace(/_/g, ' '));

  return (
    <div className="h-full p-4" data-testid="workflow-runs-page">
      <SettingsTabbedPage
        title={t('flows.allRuns.title')}
        description={t('flows.allRuns.description')}>
        {/* No wrapper padding: `SettingsTabbedPage`'s body already renders
            `min-h-full pb-4 pt-4`, so a `pt-4` here double-pads the top. */}
        <div>
          {pageLoading ? (
            <CenteredLoadingState label={t('flows.allRuns.loading')} />
          ) : pageError ? (
            <ErrorBanner message={pageError} />
          ) : runs.length === 0 ? (
            <p
              className="py-8 text-center text-sm text-content-muted"
              data-testid="workflow-runs-empty">
              {t('flows.allRuns.empty')}
            </p>
          ) : (
            <ul
              className="divide-y divide-line overflow-hidden rounded-2xl border border-line bg-surface"
              data-testid="workflow-runs-list">
              {runs.map(run => {
                const displayStatus = resolveDisplayStatus(run, pendingRunIds);
                return (
                  <li key={run.id}>
                    <button
                      type="button"
                      data-testid={`workflow-run-${run.id}`}
                      onClick={() => navigate(`/flows/${run.flow_id}`)}
                      className="flex w-full items-center gap-3 p-3 text-left hover:bg-surface-hover">
                      <FlowRunStatus
                        status={displayStatus}
                        label={statusLabel(displayStatus)}
                        className="shrink-0 text-[11px]"
                      />
                      <span className="min-w-0 flex-1 truncate text-sm font-medium text-content">
                        {flowNames[run.flow_id] ?? t('flows.allRuns.unknownWorkflow')}
                      </span>
                      <span className="shrink-0 text-[11px] text-content-faint">
                        {new Date(run.started_at).toLocaleString()}
                      </span>
                    </button>
                    {run.error && (
                      <p className="px-3 pb-2 text-[11px] text-coral-600 dark:text-coral-300">
                        {run.error}
                      </p>
                    )}
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      </SettingsTabbedPage>
    </div>
  );
}
