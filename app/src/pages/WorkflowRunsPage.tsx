/**
 * WorkflowRunsPage — the aggregate "All runs" view: every workflow's runs across
 * the whole `flows` domain, newest first, backed by the `flows_list_all_runs`
 * core RPC. Each row links back to its workflow's canvas. Stays live via
 * {@link useFlowRunsLiveRefresh} while any listed run is still active, via a
 * lightweight `refetchRuns` (re-fetches just the runs, not `listFlows()` too)
 * so a run doesn't sit on "Running" until the user reloads the page.
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import PanelPage from '../components/layout/PanelPage';
import { CenteredLoadingState, ErrorBanner } from '../components/ui/LoadingState';
import { useFlowRunsLiveRefresh } from '../hooks/useFlowRunsLiveRefresh';
import { useT } from '../lib/i18n/I18nContext';
import {
  type Flow,
  type FlowRun,
  type FlowRunStatus,
  listAllFlowRuns,
  listFlows,
} from '../services/api/flowsApi';

const log = debug('app:flows:runs-page');

const STATUS_CLASS: Record<FlowRunStatus, string> = {
  running: 'bg-primary-500/15 text-primary-600 dark:text-primary-300',
  completed: 'bg-sage-500/15 text-sage-700 dark:text-sage-300',
  completed_with_warnings: 'bg-amber-500/15 text-amber-700 dark:text-amber-300',
  pending_approval: 'bg-amber-500/15 text-amber-700 dark:text-amber-300',
  failed: 'bg-coral-500/15 text-coral-700 dark:text-coral-300',
  cancelled: 'bg-content-faint/15 text-content-secondary',
};

export default function WorkflowRunsPage() {
  const { t } = useT();
  const navigate = useNavigate();
  const [runs, setRuns] = useState<FlowRun[]>([]);
  const [flowNames, setFlowNames] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [allRuns, flows] = await Promise.all([listAllFlowRuns(), listFlows()]);
      const names: Record<string, string> = {};
      flows.forEach((f: Flow) => {
        names[f.id] = f.name;
      });
      setRuns(allRuns);
      setFlowNames(names);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // Lighter-weight than `load` — only re-fetches the runs (not `listFlows()`
  // too), since flow names rarely change mid-run and re-fetching them on
  // every live-refresh tick would be wasted work.
  const refetchRuns = useCallback(() => {
    listAllFlowRuns()
      .then(setRuns)
      .catch(err => {
        // Best-effort background refresh — a transient failure here shouldn't
        // clobber the page's existing error/loading state from `load`.
        log('refetchRuns failed: %o', err);
      });
  }, []);

  useFlowRunsLiveRefresh(runs, refetchRuns);

  const statusLabel = (status: FlowRunStatus) =>
    t(`flows.allRuns.status.${status}`, status.replace(/_/g, ' '));

  return (
    <PanelPage
      testId="workflow-runs-page"
      title={t('flows.allRuns.title')}
      description={t('flows.allRuns.description')}>
      <div className="p-4">
        {loading ? (
          <CenteredLoadingState label={t('flows.allRuns.loading')} />
        ) : error ? (
          <ErrorBanner message={error} />
        ) : runs.length === 0 ? (
          <p
            className="py-8 text-center text-sm text-content-muted"
            data-testid="workflow-runs-empty">
            {t('flows.allRuns.empty')}
          </p>
        ) : (
          <ul
            className="divide-y divide-line rounded-xl border border-line"
            data-testid="workflow-runs-list">
            {runs.map(run => (
              <li key={run.id}>
                <button
                  type="button"
                  data-testid={`workflow-run-${run.id}`}
                  onClick={() => navigate(`/flows/${run.flow_id}`)}
                  className="flex w-full items-center gap-3 p-3 text-left hover:bg-surface-hover">
                  <span
                    className={`flex-shrink-0 rounded-full px-2 py-0.5 text-[11px] font-medium ${STATUS_CLASS[run.status]}`}>
                    {statusLabel(run.status)}
                  </span>
                  <span className="min-w-0 flex-1 truncate text-sm font-medium text-content">
                    {flowNames[run.flow_id] ?? t('flows.allRuns.unknownWorkflow')}
                  </span>
                  <span className="flex-shrink-0 text-[11px] text-content-faint">
                    {new Date(run.started_at).toLocaleString()}
                  </span>
                </button>
                {run.error && (
                  <p className="px-3 pb-2 text-[11px] text-coral-600 dark:text-coral-300">
                    {run.error}
                  </p>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>
    </PanelPage>
  );
}
