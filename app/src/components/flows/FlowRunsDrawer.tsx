/**
 * FlowRunsDrawer (issue B5a.1)
 * ----------------------------
 *
 * Right-side drawer listing a flow's run history, opened from the
 * "View runs" action on {@link FlowListRow}. Drawer chrome mirrors
 * `FlowRunInspectorDrawer`/`SubagentDrawer` (fixed overlay + backdrop-click-
 * to-close + Escape-to-close via `useDismissLayer`) so it renders as a fixed
 * overlay regardless of where the parent mounts it.
 *
 * Data loads via `useFlowRunsQuery` on open, then stays live via
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
import { useCallback, useEffect, useState } from 'react';

import { useDismissLayer } from '../../hooks/useDismissLayer';
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
function FlowRunsDrawer({ flowId, flowName, onClose, onFixWithAgent }: Props) {
  const { t } = useT();
  const { runs, loading, error, refreshSilently } = useFlowRunsQuery({
    scope: { kind: 'flow', flowId },
  });
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);

  useEffect(() => {
    setSelectedRunId(null);
  }, [flowId]);

  const handleRunFinished = useCallback(() => void refreshSilently(), [refreshSilently]);
  const handleRunStarted = useCallback(() => void refreshSilently(), [refreshSilently]);
  useFlowRunsLiveRefresh(runs, refreshSilently);
  useFlowRunFinished(handleRunFinished, flowId);
  // Unconditional (unlike useFlowRunsLiveRefresh, which is gated on an
  // already-active run) — fills the empty-list gap ("No runs yet") that hook
  // can't reach, so the very first run shows up as "Running" instantly
  // instead of waiting for a manual refresh (issue B35).
  useFlowRunStarted(handleRunStarted, flowId);
  const pendingRunIds = useRunsPendingApprovalSet(runs);

  const dismissalEnabled = flowId !== null && selectedRunId === null;
  const dismiss = () => {
    log('dismiss: closing flowId=%s', flowId);
    onClose();
  };
  const { layerRef, onPointerDownCapture } = useDismissLayer({
    onDismiss: dismiss,
    enabled: dismissalEnabled,
  });

  if (!flowId) return null;

  const title = flowName
    ? t('flows.runs.title').replace('{name}', flowName)
    : t('flows.runs.titleFallback');

  return (
    <>
      <div
        className="fixed inset-0 z-50 flex justify-end"
        data-testid="flow-runs-drawer"
        onPointerDownCapture={onPointerDownCapture}>
        {/* Backdrop */}
        <button
          type="button"
          aria-label={t('conversations.subagent.close')}
          data-testid="flow-runs-backdrop"
          className="absolute inset-0 bg-stone-900/30 dark:bg-black/50"
          onClick={event => {
            // Keyboard and assistive activation produces a click without a
            // preceding pointer event. Pointer clicks were already handled
            // during capture and carry a non-zero detail.
            if (event.detail === 0 && dismissalEnabled) dismiss();
          }}
        />
        <aside
          ref={element => {
            layerRef.current = element;
          }}
          className="relative flex h-full w-full max-w-md flex-col bg-surface shadow-xl">
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
              <div data-testid="flow-runs-loading">
                <CenteredLoadingState label={t('flows.runs.loading')} />
              </div>
            )}

            {error && (
              <div data-testid="flow-runs-error">
                <ErrorBanner>
                  {t('flows.runs.loadError')}: {error}
                </ErrorBanner>
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
                        <FlowRunStatus
                          status={displayStatus}
                          label={flowRunStatusLabel(displayStatus, t)}
                          presentation="dot"
                          testId={`flow-run-row-dot-${run.id}`}
                        />
                        <FlowRunStatus
                          status={displayStatus}
                          label={flowRunStatusLabel(displayStatus, t)}
                          className="shrink-0"
                        />
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
