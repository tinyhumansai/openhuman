/**
 * IntelligenceOrchestrationTab (#3375)
 * ------------------------------------
 *
 * The Workflows / multi-agent orchestration surface in the Intelligence command
 * center. Lets the user:
 *   - browse runnable workflow definitions (`workflow_run_list_definitions`),
 *   - start the parallel-research workflow (`workflow_run_start`) — gated behind
 *     an explicit approval card for high-cost / high-concurrency definitions,
 *   - drill into a run's phase progress, child agent refs, and final synthesis,
 *   - stop / resume a run.
 *
 * Progress is poll-based (the engine emits no socket events yet): once a run is
 * selected and non-terminal, this tab polls `workflow_run_get` on an interval
 * and feeds fresh snapshots to {@link WorkflowRunDetail}.
 *
 * This is a sibling of {@link WorkflowsTab} (which manages SKILL.md authoring) —
 * the two are deliberately distinct primitives: that tab is reusable procedures,
 * this tab is declarative multi-agent runs.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  assessWorkflowCost,
  type WorkflowDefinition,
  type WorkflowRun,
  workflowRunsApi,
} from '../../services/api/workflowRunsApi';
import Button from '../ui/Button';
import { SAFETY_TIER_KEY, WorkflowRunApprovalCard } from './WorkflowRunApprovalCard';
import WorkflowRunDetail from './WorkflowRunDetail';

const log = debug('intelligence:orchestration');

/** How often to poll a selected, non-terminal run for progress. */
const POLL_INTERVAL_MS = 2000;

const TERMINAL = new Set(['completed', 'failed', 'cancelled', 'interrupted']);

function isTerminal(run: WorkflowRun | null): boolean {
  return run !== null && TERMINAL.has(run.status);
}

export default function IntelligenceOrchestrationTab() {
  const { t } = useT();

  const [definitions, setDefinitions] = useState<WorkflowDefinition[]>([]);
  const [runs, setRuns] = useState<WorkflowRun[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Start flow state.
  const [startTarget, setStartTarget] = useState<WorkflowDefinition | null>(null);
  const [question, setQuestion] = useState('');
  const [starting, setStarting] = useState(false);
  const [startError, setStartError] = useState<string | null>(null);

  // Drill-in state.
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [selectedRun, setSelectedRun] = useState<WorkflowRun | null>(null);
  const [controlBusy, setControlBusy] = useState(false);

  const mountedRef = useRef(true);

  const load = useCallback(async () => {
    log('load: entry');
    setError(null);
    try {
      const [defs, runList] = await Promise.all([
        workflowRunsApi.listDefinitions(),
        workflowRunsApi.listRuns({ limit: 50 }),
      ]);
      if (!mountedRef.current) return;
      setDefinitions(defs);
      setRuns(runList);
      log('load: defs=%d runs=%d', defs.length, runList.length);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('load: error %s', msg);
      if (mountedRef.current) setError(msg);
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    // Defer the first fetch a tick so the loading state paints before `load`
    // touches state (mirrors IntelligenceAgentWorkTab; avoids a synchronous
    // setState inside the effect body).
    const handle = window.setTimeout(() => void load(), 0);
    return () => {
      window.clearTimeout(handle);
      mountedRef.current = false;
    };
  }, [load]);

  // Merge a fresh run snapshot into the runs list (newest data wins).
  const upsertRun = useCallback((run: WorkflowRun) => {
    setRuns(prev => {
      const idx = prev.findIndex(r => r.id === run.id);
      if (idx === -1) return [run, ...prev];
      const next = prev.slice();
      next[idx] = run;
      return next;
    });
  }, []);

  // ---- Polling loop for the selected run -------------------------------
  useEffect(() => {
    if (!selectedRunId) return;
    if (isTerminal(selectedRun)) return; // nothing left to poll

    let cancelled = false;
    let inFlight = false;
    let handle: number | undefined;

    const tick = async () => {
      if (cancelled || inFlight) return;
      inFlight = true;
      try {
        const run = await workflowRunsApi.getRun(selectedRunId);
        if (cancelled || !mountedRef.current || !run) return;
        setSelectedRun(run);
        upsertRun(run);
        if (!isTerminal(run)) {
          handle = window.setTimeout(() => void tick(), POLL_INTERVAL_MS);
        }
      } catch (err) {
        log('poll error %s', err instanceof Error ? err.message : String(err));
      } finally {
        inFlight = false;
      }
    };

    void tick();
    return () => {
      cancelled = true;
      if (handle !== undefined) window.clearTimeout(handle);
    };
  }, [selectedRunId, selectedRun, upsertRun]);

  const openRun = useCallback((run: WorkflowRun) => {
    log('openRun id=%s', run.id);
    setSelectedRunId(run.id);
    setSelectedRun(run);
  }, []);

  // ---- Start flow ------------------------------------------------------
  const beginStart = useCallback((def: WorkflowDefinition) => {
    log('beginStart definitionId=%s', def.id);
    setStartTarget(def);
    setQuestion('');
    setStartError(null);
  }, []);

  const cancelStart = useCallback(() => {
    setStartTarget(null);
    setStarting(false);
    setStartError(null);
  }, []);

  const doStart = useCallback(async () => {
    if (!startTarget) return;
    setStarting(true);
    setStartError(null);
    const trimmed = question.trim();
    try {
      const run = await workflowRunsApi.startRun({
        definitionId: startTarget.id,
        input: trimmed.length > 0 ? { question: trimmed } : undefined,
      });
      if (!mountedRef.current) return;
      log('started runId=%s', run.id);
      upsertRun(run);
      setStartTarget(null);
      openRun(run);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('start error %s', msg);
      if (mountedRef.current) setStartError(msg);
    } finally {
      if (mountedRef.current) setStarting(false);
    }
  }, [startTarget, question, upsertRun, openRun]);

  // ---- Stop / Resume ---------------------------------------------------
  const handleStop = useCallback(
    async (id: string) => {
      setControlBusy(true);
      try {
        const run = await workflowRunsApi.stopRun(id);
        if (run && mountedRef.current) {
          setSelectedRun(run);
          upsertRun(run);
        }
      } catch (err) {
        log('stop error %s', err instanceof Error ? err.message : String(err));
      } finally {
        if (mountedRef.current) setControlBusy(false);
      }
    },
    [upsertRun]
  );

  const handleResume = useCallback(
    async (id: string) => {
      setControlBusy(true);
      try {
        const run = await workflowRunsApi.resumeRun(id);
        if (mountedRef.current) {
          setSelectedRun(run);
          upsertRun(run);
        }
      } catch (err) {
        log('resume error %s', err instanceof Error ? err.message : String(err));
      } finally {
        if (mountedRef.current) setControlBusy(false);
      }
    },
    [upsertRun]
  );

  if (loading) {
    return (
      <div className="flex items-center justify-center py-10 text-content-faint">
        <div className="mr-2 h-4 w-4 animate-spin rounded-full border-2 border-ocean-500 border-t-transparent" />
        <span className="text-sm">{t('orchestration.loading')}</span>
      </div>
    );
  }

  if (error) {
    return (
      <div className="space-y-3">
        <div className="rounded-xl border border-coral-200 bg-coral-50 px-4 py-3 text-sm text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
          {t('orchestration.failedToLoad')}: {error}
        </div>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => {
            setLoading(true);
            void load();
          }}>
          {t('common.retry')}
        </Button>
      </div>
    );
  }

  return (
    <div className="space-y-6" data-testid="orchestration-tab">
      <p className="text-xs text-content-faint">{t('orchestration.subtitle')}</p>

      {/* Definitions catalog */}
      <section className="space-y-2">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-content-muted">
          {t('orchestration.definitions')}
        </h3>
        {definitions.length === 0 ? (
          <div className="rounded-xl border border-dashed border-line py-8 text-center text-sm text-content-faint">
            {t('orchestration.noDefinitions')}
          </div>
        ) : (
          <ul className="space-y-2" data-testid="orchestration-definitions">
            {definitions.map(def => {
              const assessment = assessWorkflowCost(def);
              const isStarting = startTarget?.id === def.id;
              return (
                <li
                  key={def.id}
                  data-testid={`orchestration-definition-${def.id}`}
                  className="rounded-xl border border-line bg-surface p-3">
                  <div className="flex flex-wrap items-start justify-between gap-2">
                    <div className="min-w-0">
                      <div className="flex flex-wrap items-center gap-1.5">
                        <span className="text-sm font-medium text-content">{def.name}</span>
                        <span className="rounded-md border border-line px-1.5 py-0.5 text-[10px] font-medium text-content-muted">
                          {t(SAFETY_TIER_KEY[def.safetyTier])}
                        </span>
                        {assessment.requiresApproval && (
                          <span
                            data-testid={`orchestration-approval-badge-${def.id}`}
                            className="rounded-md border border-amber-300 bg-amber-50 px-1.5 py-0.5 text-[10px] font-medium text-amber-700 dark:border-amber-500/40 dark:bg-amber-500/10 dark:text-amber-300">
                            {t('orchestration.approvalRequired')}
                          </span>
                        )}
                      </div>
                      <p className="mt-1 text-xs text-content-muted">{def.description}</p>
                    </div>
                    {!isStarting && (
                      <Button
                        variant="primary"
                        size="sm"
                        data-testid={`orchestration-start-${def.id}`}
                        onClick={() => beginStart(def)}
                        className="flex-none shadow-soft">
                        {t('orchestration.start')}
                      </Button>
                    )}
                  </div>

                  {/* Inline start panel */}
                  {isStarting && (
                    <div className="mt-3 space-y-3 border-t border-line-subtle pt-3">
                      <label className="block text-xs font-medium text-content-secondary">
                        {t('orchestration.questionLabel')}
                        <textarea
                          data-testid="orchestration-question"
                          value={question}
                          onChange={e => setQuestion(e.target.value)}
                          rows={2}
                          placeholder={t('orchestration.questionPlaceholder')}
                          className="mt-1 w-full rounded-lg border border-line bg-surface px-2 py-1.5 text-sm text-content focus:border-primary-500 focus:outline-none"
                        />
                      </label>

                      {startError && (
                        <p className="text-xs text-coral-600 dark:text-coral-300">⚠ {startError}</p>
                      )}

                      {assessment.requiresApproval ? (
                        <WorkflowRunApprovalCard
                          definition={def}
                          reasons={assessment.reasons}
                          starting={starting}
                          onApprove={() => void doStart()}
                          onCancel={cancelStart}
                        />
                      ) : (
                        <div className="flex items-center gap-2">
                          <Button
                            variant="primary"
                            size="sm"
                            data-testid="orchestration-confirm-start"
                            disabled={starting}
                            onClick={() => void doStart()}
                            className="shadow-soft">
                            {starting
                              ? t('orchestration.starting')
                              : t('orchestration.confirmStart')}
                          </Button>
                          <Button
                            variant="secondary"
                            size="sm"
                            onClick={cancelStart}
                            disabled={starting}>
                            {t('orchestration.approval.cancel')}
                          </Button>
                        </div>
                      )}
                    </div>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </section>

      {/* Selected run drill-in */}
      {selectedRun && (
        <section className="space-y-2" data-testid="orchestration-selected-run">
          <div className="flex items-center justify-between">
            <h3 className="text-xs font-semibold uppercase tracking-wide text-content-muted">
              {t('orchestration.runProgress')}
            </h3>
            <button
              type="button"
              data-testid="orchestration-close-run"
              onClick={() => {
                setSelectedRunId(null);
                setSelectedRun(null);
              }}
              className="text-[11px] text-content-faint hover:text-content-secondary">
              {t('orchestration.close')}
            </button>
          </div>
          <WorkflowRunDetail
            definition={definitions.find(d => d.id === selectedRun.definitionId)}
            run={selectedRun}
            busy={controlBusy}
            onStop={id => void handleStop(id)}
            onResume={id => void handleResume(id)}
          />
        </section>
      )}

      {/* Recent runs */}
      <section className="space-y-2">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-content-muted">
          {t('orchestration.recentRuns')}
        </h3>
        {runs.length === 0 ? (
          <div className="rounded-xl border border-dashed border-line py-8 text-center text-sm text-content-faint">
            {t('orchestration.noRuns')}
          </div>
        ) : (
          <ul
            className="divide-y divide-line-subtle overflow-hidden rounded-xl border border-line bg-surface dark:divide-neutral-800"
            data-testid="orchestration-runs">
            {runs.map(run => {
              const def = definitions.find(d => d.id === run.definitionId);
              return (
                <li key={run.id}>
                  <button
                    type="button"
                    data-testid={`orchestration-run-${run.id}`}
                    onClick={() => openRun(run)}
                    className={`flex w-full items-center justify-between gap-2 p-3 text-left hover:bg-surface-hover ${
                      run.id === selectedRunId ? 'bg-surface-muted' : ''
                    }`}>
                    <span className="min-w-0">
                      <span className="block truncate text-sm font-medium text-content">
                        {def?.name ?? run.definitionId}
                      </span>
                      <span className="font-mono text-[10px] text-content-faint">{run.id}</span>
                    </span>
                    <span className="flex-none text-[11px] text-content-faint">
                      {t(`orchestration.runStatus.${run.status}`)}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </section>
    </div>
  );
}
