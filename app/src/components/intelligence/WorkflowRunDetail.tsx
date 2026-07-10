/**
 * WorkflowRunDetail (#3375)
 * -------------------------
 *
 * Drill-in view for a single workflow run. Given a definition (for phase
 * ordering + labels) and the latest run snapshot, it renders:
 *   - the run status header,
 *   - an ordered phase timeline driven by `run.phaseStates`,
 *   - each phase's child agent refs (orchestration id + agent id + output),
 *   - Stop / Resume controls wired to the engine,
 *   - the final synthesized report once the run completes.
 *
 * The component is presentational + control-only: the parent (Orchestration
 * tab) owns the polling loop and passes a fresh `run` each tick. Stop / Resume
 * delegate to callbacks so the parent can refresh its run list too.
 */
import debug from 'debug';
import React, { useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type WorkflowDefinition,
  type WorkflowPhaseState,
  type WorkflowPhaseStatus,
  type WorkflowRun,
  type WorkflowRunStatus,
} from '../../services/api/workflowRunsApi';
import Button from '../ui/Button';

const log = debug('intelligence:workflow-detail');

/** Accent classes per run status (semantic palette from tailwind.config.js). */
const RUN_STATUS_ACCENT: Record<WorkflowRunStatus, string> = {
  pending: 'border-line bg-surface-muted text-content-secondary',
  running:
    'border-ocean-200 bg-ocean-50 text-ocean-700 dark:border-ocean-500/30 dark:bg-ocean-500/10 dark:text-ocean-300',
  completed:
    'border-sage-200 bg-sage-50 text-sage-700 dark:border-sage-500/30 dark:bg-sage-500/10 dark:text-sage-300',
  failed:
    'border-coral-200 bg-coral-50 text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300',
  cancelled: 'border-line bg-surface-muted text-content-secondary',
  interrupted:
    'border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-300',
};

const RUN_STATUS_KEY: Record<WorkflowRunStatus, string> = {
  pending: 'orchestration.runStatus.pending',
  running: 'orchestration.runStatus.running',
  completed: 'orchestration.runStatus.completed',
  failed: 'orchestration.runStatus.failed',
  cancelled: 'orchestration.runStatus.cancelled',
  interrupted: 'orchestration.runStatus.interrupted',
};

const PHASE_STATUS_KEY: Record<WorkflowPhaseStatus, string> = {
  pending: 'orchestration.phaseStatus.pending',
  running: 'orchestration.phaseStatus.running',
  completed: 'orchestration.phaseStatus.completed',
  failed: 'orchestration.phaseStatus.failed',
};

/** Glyph per phase status — color comes from the surrounding classes. */
const PHASE_STATUS_DOT: Record<WorkflowPhaseStatus, string> = {
  pending: 'bg-surface-strong',
  running: 'bg-ocean-500 animate-pulse',
  completed: 'bg-sage-500',
  failed: 'bg-coral-500',
};

const TERMINAL_STATUSES: WorkflowRunStatus[] = ['completed', 'failed', 'cancelled', 'interrupted'];

interface Props {
  definition: WorkflowDefinition | undefined;
  run: WorkflowRun;
  /** True while a stop/resume RPC is in flight. */
  busy?: boolean;
  onStop: (id: string) => void;
  onResume: (id: string) => void;
}

export const WorkflowRunDetail: React.FC<Props> = ({
  definition,
  run,
  busy = false,
  onStop,
  onResume,
}) => {
  const { t } = useT();
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});

  // Phase order: lead with the definition's declared order, then append any
  // runtime-only phases the run reports but the definition doesn't list (so a
  // run still renders its full progress during definition/version drift).
  const declaredPhaseNames = definition?.phases.map(p => p.name) ?? [];
  const declaredSet = new Set(declaredPhaseNames);
  const runtimeOnlyPhaseNames = Object.keys(run.phaseStates).filter(name => !declaredSet.has(name));
  const phaseNames = [...declaredPhaseNames, ...runtimeOnlyPhaseNames];

  const isRunning = run.status === 'running' || run.status === 'pending';
  const canResume = run.status === 'interrupted';

  const toggle = (name: string) => setExpanded(prev => ({ ...prev, [name]: !prev[name] }));

  return (
    <div className="space-y-4" data-testid="workflow-run-detail">
      {/* Header: status + controls */}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <span
            data-testid="workflow-run-status"
            className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs font-medium ${RUN_STATUS_ACCENT[run.status]}`}>
            {run.status === 'running' && (
              <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-ocean-500" />
            )}
            {t(RUN_STATUS_KEY[run.status])}
          </span>
          <span className="font-mono text-[11px] text-content-faint">{run.id}</span>
        </div>

        <div className="flex items-center gap-2">
          {isRunning && (
            <Button
              variant="secondary"
              tone="danger"
              size="sm"
              data-testid="workflow-run-stop"
              disabled={busy}
              onClick={() => {
                log('stop id=%s', run.id);
                onStop(run.id);
              }}>
              {t('orchestration.detail.stop')}
            </Button>
          )}
          {canResume && (
            <button
              type="button"
              data-testid="workflow-run-resume"
              disabled={busy}
              onClick={() => {
                log('resume id=%s', run.id);
                onResume(run.id);
              }}
              className="rounded-lg border border-ocean-300 bg-surface px-3 py-1.5 text-xs font-medium text-ocean-700 hover:bg-ocean-50 disabled:opacity-50 dark:border-ocean-700 dark:text-ocean-300 dark:hover:bg-ocean-900/40">
              {t('orchestration.detail.resume')}
            </button>
          )}
        </div>
      </div>

      {/* Phase timeline */}
      <ol className="space-y-2" data-testid="workflow-phase-list">
        {phaseNames.map(name => {
          const phaseDef = definition?.phases.find(p => p.name === name);
          const state: WorkflowPhaseState = run.phaseStates[name] ?? {
            status: 'pending',
            outputs: [],
          };
          const isOpen = expanded[name] ?? false;
          const hasOutputs = state.outputs.length > 0;
          return (
            <li
              key={name}
              data-testid={`workflow-phase-${name}`}
              className="rounded-xl border border-line bg-surface">
              <button
                type="button"
                onClick={() => toggle(name)}
                className="flex w-full items-center justify-between gap-2 px-3 py-2 text-left">
                <span className="flex min-w-0 items-center gap-2">
                  <span
                    className={`h-2 w-2 flex-none rounded-full ${PHASE_STATUS_DOT[state.status]}`}
                  />
                  <span className="truncate text-sm font-medium text-content">
                    {phaseDef?.name ?? name}
                  </span>
                  <span
                    data-testid={`workflow-phase-status-${name}`}
                    className="rounded-md border border-line px-1.5 py-0.5 text-[10px] font-medium text-content-muted">
                    {t(PHASE_STATUS_KEY[state.status])}
                  </span>
                </span>
                <span className="flex flex-none items-center gap-2 text-[11px] text-content-faint">
                  {hasOutputs && (
                    <span data-testid={`workflow-phase-count-${name}`}>
                      {state.outputs.length} {t('orchestration.detail.agents')}
                    </span>
                  )}
                  <span aria-hidden>{isOpen ? '▾' : '▸'}</span>
                </span>
              </button>

              {phaseDef?.description && (
                <p className="px-3 pb-1 text-xs text-content-muted">{phaseDef.description}</p>
              )}

              {state.status === 'failed' && state.reason && (
                <p className="mx-3 mb-2 rounded-md bg-coral-50 px-2 py-1 text-xs text-coral-700 dark:bg-coral-500/10 dark:text-coral-300">
                  {state.reason}
                </p>
              )}

              {/* Child agent refs for this phase */}
              {isOpen && hasOutputs && (
                <ul className="space-y-2 px-3 pb-3" data-testid={`workflow-phase-outputs-${name}`}>
                  {state.outputs.map((out, idx) => (
                    <li
                      key={`${out.orchestrationId}-${idx}`}
                      className="rounded-lg border border-line-subtle bg-surface-muted p-2">
                      <div className="flex flex-wrap items-center gap-1.5">
                        <span className="text-xs font-medium text-content-secondary">
                          {out.agentId}
                        </span>
                        <span className="font-mono text-[10px] text-content-faint">
                          {out.orchestrationId}
                        </span>
                      </div>
                      {out.output && (
                        <p className="mt-1 whitespace-pre-wrap break-words text-xs leading-snug text-content-secondary">
                          {out.output}
                        </p>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </li>
          );
        })}
      </ol>

      {/* Child agent refs summary (full run-level list) */}
      {run.childRunIds.length > 0 && (
        <div className="text-[11px] text-content-faint" data-testid="workflow-child-refs">
          {t('orchestration.detail.childRefs')}: {run.childRunIds.length}
        </div>
      )}

      {/* Final synthesis */}
      {run.summary && TERMINAL_STATUSES.includes(run.status) && (
        <div
          data-testid="workflow-run-summary"
          className="rounded-xl border border-sage-200 bg-sage-50 p-3 dark:border-sage-500/30 dark:bg-sage-500/10">
          <p className="mb-1 text-xs font-semibold text-sage-800 dark:text-sage-200">
            {t('orchestration.detail.synthesis')}
          </p>
          <p className="whitespace-pre-wrap break-words text-sm leading-snug text-content-secondary">
            {run.summary}
          </p>
        </div>
      )}
    </div>
  );
};

export default WorkflowRunDetail;
