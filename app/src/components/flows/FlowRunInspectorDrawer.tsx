/**
 * FlowRunInspectorDrawer (issue B3b)
 * ----------------------------------
 *
 * Right-side drawer showing a single durable `tinyflows` run's status + step
 * timeline, opened from the "View run" action on {@link FlowApprovalCard}.
 * Drawer chrome mirrors `pages/conversations/components/SubagentDrawer.tsx`
 * (fixed overlay + backdrop-click-to-close + Escape-to-close) so it renders
 * as a fixed overlay regardless of where the parent mounts it in the DOM.
 *
 * Data comes from {@link useFlowRunPoller}, which polls
 * `openhuman.flows_get_run` every 2s until the run reaches a terminal status
 * (`completed`/`failed`) — `pending_approval` keeps polling since the run can
 * still be resumed elsewhere.
 *
 * `FlowRunStep` is lean by design (`node_id` + `output` + optional `port`
 * only — no per-step status/timing), so each step renders as a plain label
 * + collapsible output, not a graduated status timeline. Status-dot/pill
 * visual language borrows from `components/intelligence/WorkflowRunDetail.tsx`
 * (`RUN_STATUS_ACCENT`/`PHASE_STATUS_DOT`) and
 * `pages/conversations/components/ToolTimelineBlock.tsx` (`StatusTag`) —
 * dots, not progress bars (project rule).
 */
import debug from 'debug';

import { useEscapeKey } from '../../hooks/useEscapeKey';
import { useFlowRunPoller } from '../../hooks/useFlowRunPoller';
import { type FlowNodeRunStatus, useFlowRunProgress } from '../../hooks/useFlowRunProgress';
import { type FlowRunItem, normalizeItems } from '../../lib/flows/runItems';
import { useT } from '../../lib/i18n/I18nContext';
import type { FlowRunStatus, FlowRunStep } from '../../services/api/flowsApi';
import Button from '../ui/Button';
import { RunItemDataBrowser } from './RunItemDataBrowser';

/**
 * Context handed to the "Fix with agent" action (Phase 5c) so the canvas
 * copilot can open preloaded with the failed run. `flowId` routes to the flow's
 * canvas; the rest seeds the repair prompt.
 */
export interface FlowRepairRequest {
  flowId: string;
  runId: string;
  error?: string | null;
  failingNodeIds?: string[];
}

const log = debug('flows:run-inspector-drawer');

/**
 * Accent classes per run status (semantic palette from tailwind.config.js).
 * Exported so {@link FlowRunsDrawer} (issue B5a.1) can reuse the same
 * status-pill visual language for its run-history rows instead of
 * duplicating the mapping.
 */
export const FLOW_RUN_STATUS_ACCENT: Record<FlowRunStatus, string> = {
  running:
    'border-ocean-200 bg-ocean-50 text-ocean-700 dark:border-ocean-500/30 dark:bg-ocean-500/10 dark:text-ocean-300',
  completed:
    'border-sage-200 bg-sage-50 text-sage-700 dark:border-sage-500/30 dark:bg-sage-500/10 dark:text-sage-300',
  // Settled like `completed`, but at least one step had a `=`-binding that
  // resolved to `null` (run honesty, PR2) — reuse `pending_approval`'s amber
  // so "needs a look" reads consistently across statuses.
  completed_with_warnings:
    'border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-300',
  pending_approval:
    'border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-300',
  failed:
    'border-coral-200 bg-coral-50 text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300',
  // Neutral treatment, matching `WorkflowRunDetail.tsx`'s `RUN_STATUS_ACCENT.cancelled`.
  cancelled: 'border-line bg-surface-muted text-content-secondary',
};

/** Header status dot per run status — mirrors `PHASE_STATUS_DOT`. Exported, see above. */
export const FLOW_RUN_STATUS_DOT: Record<FlowRunStatus, string> = {
  running: 'bg-ocean-500 animate-pulse',
  completed: 'bg-sage-500',
  // Settled (no pulse) — the amber signals "worth a look", not "in progress".
  completed_with_warnings: 'bg-amber-500',
  pending_approval: 'bg-amber-500 animate-pulse',
  failed: 'bg-coral-500',
  cancelled: 'bg-surface-strong',
};

/** i18n key per run status. Exported, see above. */
export const FLOW_RUN_STATUS_KEY: Record<FlowRunStatus, string> = {
  running: 'flowRuns.status.running',
  completed: 'flowRuns.status.completed',
  completed_with_warnings: 'flowRuns.status.completed_with_warnings',
  pending_approval: 'flowRuns.status.pending_approval',
  failed: 'flowRuns.status.failed',
  cancelled: 'flowRuns.status.cancelled',
};

function formatTimestamp(value: string | null | undefined): string | null {
  if (!value) return null;
  const parsed = Date.parse(value);
  if (!Number.isFinite(parsed)) return null;
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
    second: '2-digit',
  }).format(new Date(parsed));
}

/**
 * Live per-node status dot colour, keyed off the socket `flow:run_progress`
 * feed (Phase 3e). Mirrors the run-level {@link FLOW_RUN_STATUS_DOT} language:
 * ocean (running, pulsing), sage (success), coral (error). Falls back to the
 * faint dot when the node has no live status yet (the poller stays the source
 * of truth for the durable step list).
 */
const FLOW_STEP_LIVE_DOT: Record<string, string> = {
  running: 'bg-ocean-500 animate-pulse',
  success: 'bg-sage-500',
  error: 'bg-coral-500',
  failed: 'bg-coral-500',
};

function StepRow({
  step,
  index,
  liveStatus,
  inputItems,
}: {
  step: FlowRunStep;
  index: number;
  liveStatus?: FlowNodeRunStatus;
  /**
   * Normalized items of this step's *input* (the upstream step's output) so the
   * data browser can resolve `paired_item` back to a source input item.
   * Omitted for the first step, which has no upstream producer here.
   */
  inputItems?: FlowRunItem[];
}) {
  const { t } = useT();
  const items = normalizeItems(step.output);
  const dotClass = (liveStatus && FLOW_STEP_LIVE_DOT[liveStatus]) ?? 'bg-content-faint';

  return (
    <li
      data-testid={`flow-run-step-${index}`}
      className="rounded-lg border border-line bg-surface-muted p-2.5 text-xs">
      <div className="flex flex-wrap items-center gap-1.5">
        <span
          data-testid={`flow-run-step-dot-${index}`}
          className={`h-1.5 w-1.5 flex-none rounded-full ${dotClass}`}
          aria-hidden
        />
        <span className="truncate font-mono font-medium text-content-secondary">
          {step.node_id}
        </span>
        {step.port !== undefined && (
          <span
            data-testid={`flow-run-step-port-${index}`}
            className="rounded-md border border-line px-1.5 py-0.5 text-[10px] font-medium text-content-muted">
            {t('flowRuns.inspector.port')}: {step.port}
          </span>
        )}
      </div>
      {/* Null-resolution diagnostics: each config `=`-expression that resolved
          to null during this step (a wiring smell, not a hard failure). */}
      {step.diagnostics && step.diagnostics.length > 0 && (
        <div
          data-testid={`flow-run-step-diagnostics-${index}`}
          className="mt-1.5 rounded-lg border border-amber-200 bg-amber-50 px-2 py-1.5 text-[11px] text-amber-700 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-300">
          <div className="font-medium">{t('flowRuns.inspector.diagnosticsTitle')}</div>
          <ul className="mt-0.5 space-y-0.5">
            {step.diagnostics.map((diag, diagIdx) => (
              <li key={`${diag.location}-${diagIdx}`} className="break-all font-mono">
                {diag.location} ← {diag.expression}{' '}
                <span className="font-sans">{t('flowRuns.inspector.diagnosticResolvedNull')}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
      {items.length > 0 && (
        <details className="mt-1.5">
          <summary className="cursor-pointer text-[11px] font-medium text-content-faint hover:text-content-secondary">
            {t('flowRuns.inspector.output')}
          </summary>
          <div className="mt-1.5">
            <RunItemDataBrowser
              items={items}
              inputItems={inputItems}
              testIdPrefix={`flow-run-step-${index}`}
            />
          </div>
        </details>
      )}
    </li>
  );
}

interface Props {
  /** Run id (== thread_id) to inspect. Renders `null` (nothing) when absent. */
  runId: string | null;
  onClose: () => void;
  /**
   * "Fix with agent" (Phase 5c) — when provided and the run failed, a repair
   * action surfaces that hands the run context up so the host can open the
   * canvas copilot preloaded. Omitted where there's no copilot to route to.
   */
  onFixWithAgent?: (request: FlowRepairRequest) => void;
}

/**
 * Renders `null` when `runId` is `null` so the parent can mount this
 * unconditionally and just flip `runId` (same convention as
 * `SubagentDrawer`).
 */
export function FlowRunInspectorDrawer({ runId, onClose, onFixWithAgent }: Props) {
  const { t } = useT();
  const { run, loading, error } = useFlowRunPoller(runId);
  // Live per-node status overlay (Phase 3e): the socket feed makes the poller's
  // durable step list feel live without replacing it as the source of truth.
  const liveStatuses = useFlowRunProgress(runId);

  const handleFixWithAgent = () => {
    if (!run || !onFixWithAgent) return;
    // Best-effort failing-node hints from the live status feed (error/failed).
    const failingNodeIds = Object.entries(liveStatuses)
      .filter(([, status]) => status === 'error' || status === 'failed')
      .map(([nodeId]) => nodeId);
    log(
      'fix-with-agent: flow=%s run=%s failing=%d',
      run.flow_id,
      run.thread_id,
      failingNodeIds.length
    );
    onFixWithAgent({
      flowId: run.flow_id,
      runId: run.thread_id,
      error: run.error,
      failingNodeIds: failingNodeIds.length > 0 ? failingNodeIds : undefined,
    });
  };

  useEscapeKey(() => {
    log('escape: closing runId=%s', runId);
    onClose();
  }, runId !== null);

  if (!runId) return null;

  const startedAt = formatTimestamp(run?.started_at);
  const finishedAt = formatTimestamp(run?.finished_at);
  const pendingCount = run?.pending_approvals.length ?? 0;

  return (
    <div className="fixed inset-0 z-50 flex justify-end" data-testid="flow-run-inspector-drawer">
      {/* Backdrop */}
      <button
        type="button"
        aria-label={t('conversations.subagent.close')}
        data-testid="flow-run-inspector-backdrop"
        className="absolute inset-0 bg-stone-900/30 dark:bg-black/50"
        onClick={onClose}
      />
      <aside className="relative flex h-full w-full max-w-md flex-col bg-surface shadow-xl">
        {/* Header */}
        <header className="flex items-start gap-2.5 border-b border-line px-4 py-3">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="truncate font-semibold text-content">
                {t('flowRuns.inspector.title')}
              </span>
              {run && (
                <span
                  data-testid="flow-run-status-dot"
                  className={`h-2 w-2 shrink-0 rounded-full ${FLOW_RUN_STATUS_DOT[run.status]}`}
                />
              )}
            </div>
            <div className="mt-1 flex flex-wrap items-center gap-1.5 text-[11px] text-content-muted">
              {run && (
                <span
                  data-testid="flow-run-status-pill"
                  className={`inline-flex items-center rounded-full border px-2 py-0.5 font-medium ${FLOW_RUN_STATUS_ACCENT[run.status]}`}>
                  {t(FLOW_RUN_STATUS_KEY[run.status])}
                </span>
              )}
              {run && <span className="truncate font-mono">{run.flow_id}</span>}
              {run && <span className="truncate font-mono">{run.thread_id}</span>}
            </div>
          </div>
          <button
            type="button"
            data-testid="flow-run-inspector-close"
            onClick={onClose}
            aria-label={t('conversations.subagent.close')}
            className="shrink-0 rounded-full p-1.5 text-content-faint hover:bg-surface-hover hover:text-content-secondary">
            ✕
          </button>
        </header>

        <div className="flex-1 space-y-3 overflow-y-auto px-4 py-4">
          {loading && !run && (
            <div
              className="flex items-center gap-2 py-8 text-content-faint"
              data-testid="flow-run-inspector-loading">
              <div className="h-4 w-4 animate-spin rounded-full border-2 border-ocean-500 border-t-transparent" />
              <span className="text-sm">{t('flowRuns.inspector.loading')}</span>
            </div>
          )}

          {error && (
            <div
              role="alert"
              data-testid="flow-run-inspector-error"
              className="rounded-xl border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
              {t('flowRuns.inspector.loadError')}: {error}
            </div>
          )}

          {run && (
            <>
              {/* Timing */}
              <div className="text-xs text-content-muted" data-testid="flow-run-timing">
                {startedAt && (
                  <div>
                    {t('flowRuns.inspector.startedAt')}: {startedAt}
                  </div>
                )}
                {finishedAt ? (
                  <div>
                    {t('flowRuns.inspector.finishedAt')}: {finishedAt}
                  </div>
                ) : run.status === 'running' || run.status === 'pending_approval' ? (
                  <div className="animate-pulse">{t('flowRuns.inspector.running')}</div>
                ) : null}
              </div>

              {/* Error banner */}
              {run.error && (
                <div
                  role="alert"
                  data-testid="flow-run-error-banner"
                  className="rounded-xl border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
                  {t('flowRuns.inspector.error')}: {run.error}
                </div>
              )}

              {/* Repair entry point (Phase 5c): open the canvas copilot preloaded
                  with this failed run so the workflow builder can propose a fix. */}
              {run.status === 'failed' && onFixWithAgent && (
                <div>
                  <Button
                    type="button"
                    variant="primary"
                    size="sm"
                    data-testid="flow-run-fix-with-agent"
                    onClick={handleFixWithAgent}>
                    {t('flowRuns.inspector.fixWithAgent')}
                  </Button>
                </div>
              )}

              {/* Pending approvals banner */}
              {run.status === 'pending_approval' && pendingCount > 0 && (
                <div
                  data-testid="flow-run-pending-approvals-banner"
                  className="rounded-xl border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-700 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-300">
                  {t('flowRuns.inspector.pendingApprovalsCount').replace(
                    '{count}',
                    String(pendingCount)
                  )}
                </div>
              )}

              {/* Steps timeline */}
              <div>
                <h3 className="mb-1.5 text-xs font-semibold uppercase tracking-wide text-content-muted">
                  {t('flowRuns.inspector.steps')}
                </h3>
                {run.steps.length === 0 ? (
                  <p className="text-xs italic text-content-faint">
                    {t('flowRuns.inspector.noSteps')}
                  </p>
                ) : (
                  <ol className="space-y-2" data-testid="flow-run-steps">
                    {run.steps.map((step, idx) => (
                      <StepRow
                        key={`${step.node_id}-${idx}`}
                        step={step}
                        index={idx}
                        liveStatus={liveStatuses[step.node_id]}
                        inputItems={idx > 0 ? normalizeItems(run.steps[idx - 1].output) : undefined}
                      />
                    ))}
                  </ol>
                )}
              </div>
            </>
          )}
        </div>
      </aside>
    </div>
  );
}

export default FlowRunInspectorDrawer;
