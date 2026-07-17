/**
 * FlowRunsSidebar (Workflows UI redesign, Piece 3 — "runs rail") — the
 * workflow's recent runs, projected as a ~44px vertical icon+dot rail on the
 * canvas's left edge (was a ~240px column; this reclaims most of that width
 * for the canvas). Each dot is a color-coded run status with a hover/focus
 * tooltip (status + relative time); clicking one calls {@link onSelectRun} so
 * the host ({@link ../pages/FlowCanvasPage}'s `FlowEditor`) can show it in the
 * docked Run tab — this component no longer owns selection or renders the
 * inspector itself (both lifted to the host, Piece 1 of the redesign).
 *
 * An expander at the bottom opens a flyout with the full list (status pill +
 * relative time per row, same visual language as before) for anyone who wants
 * more than the compact dots. Fetches via `listFlowRuns`, with a manual
 * refresh button plus {@link useFlowRunsLiveRefresh} keeping the list itself
 * live while any run shown here is still active (no manual refresh/navigate-
 * away required).
 *
 * Rendered directly in-page by `FlowCanvasPage` (the app sidebar is hidden
 * entirely on this chromeless route).
 */
import createDebug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useFlowRunsLiveRefresh } from '../../hooks/useFlowRunsLiveRefresh';
import {
  resolveDisplayStatus,
  useRunsPendingApprovalSet,
} from '../../hooks/useRunsPendingApprovalSet';
import { useT } from '../../lib/i18n/I18nContext';
import { type FlowRun, listFlowRuns } from '../../services/api/flowsApi';
import Tooltip from '../ui/Tooltip';
import {
  FLOW_RUN_STATUS_ACCENT,
  FLOW_RUN_STATUS_DOT,
  FLOW_RUN_STATUS_KEY,
} from './FlowRunInspectorDrawer';

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

function RefreshIcon() {
  return (
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
  );
}

/** Chevron flipped by `expanded` — points away from the rail toward the flyout. */
function ChevronIcon({ expanded }: { expanded: boolean }) {
  return (
    <svg
      className={`h-3.5 w-3.5 transition-transform ${expanded ? 'rotate-180' : ''}`}
      fill="none"
      stroke="currentColor"
      viewBox="0 0 24 24"
      aria-hidden="true">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
    </svg>
  );
}

export interface FlowRunsSidebarProps {
  flowId: string;
  /** Controlled selection (Piece 1 — lifted up to `FlowEditor` so the rail and the dock's Run tab share it). */
  selectedRunId: string | null;
  onSelectRun: (runId: string) => void;
}

export default function FlowRunsSidebar({
  flowId,
  selectedRunId,
  onSelectRun,
}: FlowRunsSidebarProps) {
  const { t } = useT();
  const [runs, setRuns] = useState<FlowRun[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);

  const load = useCallback(async () => {
    log('loading runs for flow=%s', flowId);
    setLoading(true);
    setError(null);
    try {
      const result = await listFlowRuns(flowId);
      setRuns(result);
      log('loaded %d runs', result.length);
    } catch (err) {
      log('load failed: %o', err);
      setError(t('flows.runs.loadError'));
    } finally {
      setLoading(false);
    }
  }, [flowId, t]);

  useEffect(() => {
    void load();
  }, [load]);

  useFlowRunsLiveRefresh(runs, load);
  const pendingRunIds = useRunsPendingApprovalSet(runs);

  const selectRun = useCallback(
    (runId: string) => {
      onSelectRun(runId);
      setExpanded(false);
    },
    [onSelectRun]
  );

  return (
    <div
      className="relative flex h-full w-11 flex-shrink-0 flex-col items-center border-r border-line py-2"
      data-testid="flow-runs-sidebar">
      <Tooltip label={t('flows.runs.refresh')} side="right">
        <button
          type="button"
          onClick={() => void load()}
          disabled={loading}
          data-testid="flow-runs-sidebar-refresh"
          aria-label={t('flows.runs.refresh')}
          className="rounded-md p-1.5 text-content-faint transition-colors hover:bg-surface-hover hover:text-content-secondary disabled:opacity-50">
          <RefreshIcon />
        </button>
      </Tooltip>

      <div
        className="mt-2 flex min-h-0 flex-1 flex-col items-center gap-1.5 overflow-y-auto py-1"
        data-testid="flow-runs-sidebar-rail">
        {loading && runs.length === 0 && (
          <div
            data-testid="flow-runs-sidebar-loading"
            className="h-2.5 w-2.5 animate-pulse rounded-full bg-content-faint/40"
            aria-hidden="true"
          />
        )}

        {!loading && !error && runs.length === 0 && (
          <span
            data-testid="flow-runs-sidebar-empty"
            title={t('flows.runs.empty')}
            aria-label={t('flows.runs.empty')}
            className="h-2 w-2 rounded-full border border-dashed border-line"
          />
        )}

        {runs.map(run => {
          const displayStatus = resolveDisplayStatus(run, pendingRunIds);
          const label = `${t(FLOW_RUN_STATUS_KEY[displayStatus])} · ${relativeTime(run.started_at, t)}`;
          const selected = selectedRunId === run.id;
          return (
            <Tooltip key={run.id} label={label} side="right">
              <button
                type="button"
                data-testid={`flow-runs-sidebar-run-${run.id}`}
                aria-pressed={selected}
                onClick={() => selectRun(run.id)}
                className={`flex h-4 w-4 flex-shrink-0 items-center justify-center rounded-full transition-shadow ${
                  selected ? 'ring-2 ring-primary-500 ring-offset-1 ring-offset-surface' : ''
                }`}>
                <span
                  className={`h-2.5 w-2.5 rounded-full ${FLOW_RUN_STATUS_DOT[displayStatus]}`}
                  aria-hidden="true"
                />
              </button>
            </Tooltip>
          );
        })}
      </div>

      <Tooltip label={expanded ? t('flows.runs.collapse') : t('flows.runs.expand')} side="right">
        <button
          type="button"
          data-testid="flow-runs-sidebar-expand"
          aria-expanded={expanded}
          aria-label={expanded ? t('flows.runs.collapse') : t('flows.runs.expand')}
          onClick={() => setExpanded(e => !e)}
          className="mt-1 rounded-md p-1.5 text-content-faint transition-colors hover:bg-surface-hover hover:text-content-secondary">
          <ChevronIcon expanded={expanded} />
        </button>
      </Tooltip>

      {expanded && (
        <div
          data-testid="flow-runs-sidebar-flyout"
          className="absolute left-full top-0 z-20 max-h-full w-56 overflow-y-auto rounded-xl border border-line bg-surface p-2 shadow-xl">
          <div className="px-1 pb-1.5 text-[11px] font-semibold uppercase tracking-wide text-content-faint">
            {t('flows.runs.sidebarTitle')}
          </div>

          {error && <p className="px-1 pb-1 text-xs text-coral-600 dark:text-coral-400">{error}</p>}

          {!loading && !error && runs.length === 0 && (
            <p className="px-2 py-4 text-center text-xs text-content-faint">
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
                    data-testid={`flow-runs-flyout-run-${run.id}`}
                    onClick={() => selectRun(run.id)}
                    className={`flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-surface-hover ${
                      selectedRunId === run.id ? 'bg-surface-hover' : ''
                    }`}>
                    <span
                      className={`h-2 w-2 shrink-0 rounded-full ${FLOW_RUN_STATUS_DOT[displayStatus]}`}
                      aria-hidden="true"
                    />
                    <span className="min-w-0 flex-1">
                      <span
                        className={`inline-flex items-center rounded-full border px-1.5 py-0.5 text-[10px] font-medium ${FLOW_RUN_STATUS_ACCENT[displayStatus]}`}>
                        {t(FLOW_RUN_STATUS_KEY[displayStatus])}
                      </span>
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
      )}
    </div>
  );
}
