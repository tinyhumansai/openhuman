/**
 * FlowsPage — the Workflows list page (issue B5a).
 *
 * The discoverable hub for the `flows::` domain: lists every saved
 * `Flow` (name, enabled toggle, last-run status, Run button). "New workflow"
 * (header + empty-state) opens the Phase 4a chooser — start from scratch, pick
 * a template (Phase 4c), or describe it in Chat — each of which creates a flow
 * and opens the editable canvas (`/flows/:id`). The empty state also surfaces
 * the template gallery inline so first-time users have a one-click starting
 * point.
 */
import createDebug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import EmptyStateCard from '../components/EmptyStateCard';
import FlowListRow, { type FlowListRowBusy } from '../components/flows/FlowListRow';
import { FlowPreauthorizationOverlay } from '../components/flows/FlowPreauthorizationCard';
import type { FlowRepairRequest } from '../components/flows/FlowRunInspectorDrawer';
import FlowRunsDrawer from '../components/flows/FlowRunsDrawer';
import FlowTemplateGallery from '../components/flows/FlowTemplateGallery';
import NewWorkflowModal from '../components/flows/NewWorkflowModal';
import { useCreateFlow } from '../components/flows/useCreateFlow';
import { ToastContainer } from '../components/intelligence/Toast';
import PageSectionHeader from '../components/layout/PageSectionHeader';
import PageWelcome from '../components/layout/PageWelcome';
import PanelPage from '../components/layout/PanelPage';
import { usePageWelcomeView } from '../components/layout/usePageWelcomeView';
import BetaBanner from '../components/ui/BetaBanner';
import Button from '../components/ui/Button';
import { CenteredLoadingState, ErrorBanner } from '../components/ui/LoadingState';
import { ModalShell } from '../components/ui/ModalShell';
import { useFlowChanged } from '../hooks/useFlowChanged';
import { useFlowPreauthorization } from '../hooks/useFlowPreauthorization';
import { useFlowRunFinished } from '../hooks/useFlowRunFinished';
import { FLOW_CANVAS_DRAFT_ROUTE, type FlowCanvasDraftState } from '../lib/flows/canvasDraft';
import { downloadFlowGraph } from '../lib/flows/exportFlow';
import { type FlowTemplate, templateNameKey } from '../lib/flows/templates';
import type { WorkflowGraph } from '../lib/flows/types';
import { useT } from '../lib/i18n/I18nContext';
import {
  deleteFlow,
  duplicateFlow,
  type Flow,
  importFlow,
  listFlows,
  runFlowDetached,
  setFlowEnabled,
} from '../services/api/flowsApi';
import type { ToastNotification } from '../types/intelligence';
import WorkflowDiscoveriesPage from './WorkflowDiscoveriesPage';
import WorkflowRunsPage from './WorkflowRunsPage';

const log = createDebug('app:flows');

/** How often the completion backstop refetches while a run is outstanding. */
const RUN_BACKSTOP_POLL_MS = 30_000;
/**
 * How long the backstop keeps polling for a single run before giving up —
 * comfortably past the engine's own ~600s run ceiling, so a run that is merely
 * slow is never abandoned, while one that never reports terminal cannot poll
 * indefinitely.
 */
const RUN_BACKSTOP_TTL_MS = 15 * 60_000;

/** Which action a given row currently has in flight, if any. */
type RowAction = 'toggle' | 'run';

function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export default function FlowsPage() {
  const { t } = useT();
  const navigate = useNavigate();
  const [flows, setFlows] = useState<Flow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // F-M2 fix: keyed PER FLOW ID rather than a single page-global value, so one
  // row's in-flight Toggle/Run no longer disables every other row's actions —
  // this matters most now that Run (`runFlowDetached`, F-M1) starts a run that
  // can take minutes without blocking the RPC, so a page-global lock would
  // have frozen the whole list for that long.
  const [busyByFlow, setBusyByFlow] = useState<Record<string, RowAction>>({});
  /** Run ids started from this page that have not reported terminal yet, mapped to their start time. */
  const outstandingRunsRef = useRef<Map<string, number>>(new Map());
  const setRowBusy = useCallback((flowId: string, action: RowAction | null) => {
    setBusyByFlow(prev => {
      if (action === null) {
        if (!(flowId in prev)) return prev;
        const next = { ...prev };
        delete next[flowId];
        return next;
      }
      return { ...prev, [flowId]: action };
    });
  }, []);
  const [toasts, setToasts] = useState<ToastNotification[]>([]);
  // Flow whose run history is open in `FlowRunsDrawer` (B3b's run inspector
  // then stacks on top of that when a specific run is picked). `null` keeps
  // the drawer unmounted.
  const [selectedFlowId, setSelectedFlowId] = useState<string | null>(null);
  // Whether the Phase 4a "New workflow" chooser modal is open.
  const [chooserOpen, setChooserOpen] = useState(false);
  // Create-and-open logic for the empty-state inline template gallery. (The
  // chooser modal owns its own `useCreateFlow` instance.)
  const emptyCreate = useCreateFlow();
  // Flow queued for deletion behind the confirm dialog (`null` = closed), plus
  // an in-flight flag so the confirm button can show progress + block re-entry.
  const [deleteTarget, setDeleteTarget] = useState<Flow | null>(null);
  const [deleting, setDeleting] = useState(false);

  const addToast = useCallback((toast: Omit<ToastNotification, 'id'>) => {
    setToasts(prev => [...prev, { ...toast, id: `toast-${Date.now()}-${Math.random()}` }]);
  }, []);
  const removeToast = useCallback((id: string) => {
    setToasts(prev => prev.filter(item => item.id !== id));
  }, []);

  const loadFlows = useCallback(async () => {
    log('loading flows');
    setLoading(true);
    setError(null);
    try {
      const result = await listFlows();
      setFlows(result);
      log('loaded %d flows', result.length);
    } catch (err) {
      log('load failed: %o', err);
      setError(t('flows.page.loadError'));
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    void loadFlows();
  }, [loadFlows]);

  // Refetch (silently, no spinner) whenever any flow changes underneath us —
  // e.g. an agent save_workflow — so the list never shows stale state (F6).
  useFlowChanged(
    useCallback(() => {
      log('flow:changed — refetching list');
      void listFlows()
        .then(setFlows)
        .catch(err => log('refetch failed: %o', err));
    }, [])
  );

  // Consolidated save+enable pre-authorization (Approve all / Deny). When a
  // decision settles either way, silently refresh the list so the enabled
  // toggle reflects the outcome.
  const preauth = useFlowPreauthorization({
    onSettled: useCallback(() => {
      void listFlows()
        .then(setFlows)
        .catch(err => log('post-preauth refetch failed: %o', err));
    }, []),
  });

  const handleToggle = useCallback(
    async (flow: Flow) => {
      if (busyByFlow[flow.id]) return;
      setRowBusy(flow.id, 'toggle');
      setError(null);
      log('toggle: id=%s next=%s', flow.id, !flow.enabled);
      try {
        if (flow.enabled) {
          const updated = await setFlowEnabled(flow.id, false);
          setFlows(prev => prev.map(f => (f.id === updated.id ? updated : f)));
        } else {
          // Enabling goes through the pre-authorization check: enables
          // directly when no grants are missing, otherwise surfaces the
          // consolidated card and defers the enable to "Approve all".
          const enabledNow = await preauth.beginEnable(flow.id);
          log(
            'toggle: id=%s beginEnable settled enabledNow=%s (false = preauth card shown)',
            flow.id,
            enabledNow
          );
          if (enabledNow) {
            const result = await listFlows();
            setFlows(result);
          }
        }
      } catch (err) {
        log('toggle failed: id=%s err=%o', flow.id, err);
        setError(errorMessage(err));
      } finally {
        setRowBusy(flow.id, null);
      }
    },
    [busyByFlow, preauth, setRowBusy]
  );

  // F-M1/F-M2 fix: `runFlowDetached` (`openhuman.flows_run_detached`) returns
  // as soon as the run is registered — genuinely fire-and-forget, unlike the
  // old `runFlow` (`openhuman.flows_run`) this replaced, which BLOCKED until
  // the run reached a terminal status (up to ~600s). That meant the old
  // per-row busy flag (even before it was widened to page-global) was
  // misleadingly named: "Run started" only toasted once the run had actually
  // FINISHED, and this row (and, under the page-global lock this replaces,
  // every other row too) stayed disabled for the run's whole duration.
  //
  // The row's own busy flag now only covers the brief registration round-trip
  // — `useFlowRunFinished` below refetches the list (silently) once the
  // engine actually settles, which is what now refreshes `last_run_at` /
  // `last_status`, so that signal isn't lost just because this call no longer
  // waits for it.
  const handleRun = useCallback(
    async (flow: Flow) => {
      if (busyByFlow[flow.id]) return;
      setRowBusy(flow.id, 'run');
      setError(null);
      log('run: id=%s', flow.id);
      try {
        const result = await runFlowDetached(flow.id);
        log('run: started (detached) id=%s run_id=%s', flow.id, result.run_id);
        // Arm the completion backstop below — the RPC no longer blocks until
        // the run settles, so `FlowRunFinished` is the primary signal and this
        // is the fallback if that broadcast is missed.
        outstandingRunsRef.current.set(result.run_id, Date.now());
        addToast({ type: 'success', title: t('flows.list.runStarted') });
      } catch (err) {
        log('run failed: id=%s err=%o', flow.id, err);
        setError(errorMessage(err));
      } finally {
        setRowBusy(flow.id, null);
      }
    },
    [busyByFlow, addToast, setRowBusy, t]
  );

  // Silently refetch the list whenever ANY run finishes, so a row's
  // `last_run_at` / `last_status` picks up a detached run's outcome without
  // the user having to manually refresh — the completion signal `handleRun`
  // used to get "for free" by blocking on `flows_run` until it settled.
  useFlowRunFinished(
    useCallback(event => {
      log(
        'run finished: flow=%s run=%s status=%s — refetching list',
        event.flow_id,
        event.run_id,
        event.status
      );
      outstandingRunsRef.current.delete(event.run_id);
      void listFlows()
        .then(setFlows)
        .catch(err => log('post-run-finish refetch failed: %o', err));
    }, [])
  );

  // Completion backstop for a run started from THIS page.
  //
  // `flows_run_detached` returns as soon as the run is registered, so a row's
  // `last_run_at`/`last_status` is refreshed by the `FlowRunFinished` broadcast
  // above rather than by the RPC returning. That broadcast is a plain
  // `io.emit` to currently-connected sockets with no server-side replay, so a
  // socket drop between clicking Run and the run settling (reconnect gap,
  // laptop sleep, backgrounded tab) loses it — and the row would then show a
  // stale outcome until the user navigates away and back.
  //
  // Every other run-outcome surface (`FlowRunsSidebar`, `FlowRunsDrawer`,
  // `WorkflowRunsPage`) already pairs `useFlowRunFinished` with
  // `useFlowRunsLiveRefresh` for exactly this reason; that hook can't be reused
  // verbatim here because it is typed for `FlowRun[]` and this page holds
  // `Flow[]`, so the same guarantee is rebuilt against outstanding run ids.
  //
  // Bounded on purpose: entries older than `RUN_BACKSTOP_TTL_MS` are dropped,
  // so a run that never reports terminal (process died mid-run) can't leave
  // this polling forever.
  useEffect(() => {
    const timer = setInterval(() => {
      const outstanding = outstandingRunsRef.current;
      if (outstanding.size === 0) return;
      const cutoff = Date.now() - RUN_BACKSTOP_TTL_MS;
      for (const [runId, startedAt] of outstanding) {
        if (startedAt < cutoff) {
          log('run backstop: giving up on run=%s (exceeded backstop TTL)', runId);
          outstanding.delete(runId);
        }
      }
      if (outstanding.size === 0) return;
      log('run backstop: %d run(s) outstanding — refetching list', outstanding.size);
      void listFlows()
        .then(setFlows)
        .catch(err => log('run backstop refetch failed: %o', err));
    }, RUN_BACKSTOP_POLL_MS);
    return () => clearInterval(timer);
  }, []);

  const busyFor = (flow: Flow): FlowListRowBusy => busyByFlow[flow.id] ?? null;

  const handleViewRuns = useCallback((flow: Flow) => {
    log('view runs: id=%s', flow.id);
    setSelectedFlowId(flow.id);
  }, []);

  /**
   * "Fix with agent" (Phase 5c) from a failed run's inspector: open the flow's
   * canvas with a copilot repair seed in `location.state` so the copilot opens
   * preloaded, diagnosing the failed run. Never persists — the copilot only
   * proposes.
   */
  const handleFixWithAgent = useCallback(
    (request: FlowRepairRequest) => {
      log('fix with agent: flow=%s run=%s', request.flowId, request.runId);
      setSelectedFlowId(null);
      navigate(`/flows/${request.flowId}`, {
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

  /** Opens the read-only Workflow Canvas for this flow (issue B5b.1). */
  const handleView = useCallback(
    (flow: Flow) => {
      log('view: navigating to canvas id=%s', flow.id);
      navigate(`/flows/${flow.id}`);
    },
    [navigate]
  );

  const selectedFlow = flows.find(f => f.id === selectedFlowId) ?? null;

  /** Downloads a flow's `WorkflowGraph` as a JSON file (Phase 4d export). */
  const handleExport = useCallback(
    (flow: Flow) => {
      log('export: id=%s', flow.id);
      const ok = downloadFlowGraph(flow.name, flow.graph);
      if (ok) {
        addToast({ type: 'success', title: t('flows.list.exported') });
      }
    },
    [addToast, t]
  );

  /** Duplicate a flow (server creates a disabled copy), then refresh the list. */
  const handleDuplicate = useCallback(
    async (flow: Flow) => {
      log('duplicate: id=%s', flow.id);
      setError(null);
      try {
        const copy = await duplicateFlow(flow.id);
        addToast({ type: 'success', title: t('flows.list.duplicated') });
        log('duplicate: created id=%s', copy.id);
        await loadFlows();
      } catch (err) {
        log('duplicate failed: id=%s err=%o', flow.id, err);
        setError(errorMessage(err));
      }
    },
    [addToast, loadFlows, t]
  );

  /** Confirm-gated delete: the row's Delete opens the dialog; this commits it. */
  const handleConfirmDelete = useCallback(async () => {
    if (!deleteTarget) return;
    setDeleting(true);
    setError(null);
    log('delete: confirming id=%s', deleteTarget.id);
    try {
      await deleteFlow(deleteTarget.id);
      addToast({ type: 'success', title: t('flows.list.deleted') });
      setDeleteTarget(null);
      await loadFlows();
    } catch (err) {
      log('delete failed: id=%s err=%o', deleteTarget.id, err);
      setError(errorMessage(err));
    } finally {
      setDeleting(false);
    }
  }, [deleteTarget, addToast, loadFlows, t]);

  // Hidden file input backing the header "Import" action. Clicking the button
  // opens the OS file picker; the change handler reads + imports the file.
  const importInputRef = useRef<HTMLInputElement | null>(null);

  const handleImportClick = useCallback(() => {
    log('import: opening file picker');
    importInputRef.current?.click();
  }, []);

  /**
   * Reads the picked JSON file and runs it through `flows_import` (host-side
   * migrate + validate + best-effort n8n mapping). On success, opens the
   * normalized graph on the editable canvas as an UNSAVED draft — nothing is
   * persisted until the user Saves via the canvas's existing gate. Auto-detect
   * handles native vs n8n, so no format prompt is needed.
   */
  const handleImportFile = useCallback(
    async (event: React.ChangeEvent<HTMLInputElement>) => {
      const file = event.target.files?.[0];
      // Reset the input so re-picking the same file fires `change` again.
      event.target.value = '';
      if (!file) return;
      setError(null);
      log('import: reading file name=%s size=%d', file.name, file.size);
      let parsed: unknown;
      try {
        parsed = JSON.parse(await file.text());
      } catch (err) {
        log('import: invalid JSON: %o', err);
        setError(t('flows.import.invalidFile'));
        return;
      }
      try {
        const result = await importFlow(parsed, 'auto');
        const graph = result.graph as WorkflowGraph;
        log('import: ok warnings=%d', result.warnings.length);
        const draft: FlowCanvasDraftState = {
          name: graph.name || file.name.replace(/\.[^.]+$/, ''),
          graph,
          requireApproval: true,
          importWarnings: result.warnings,
        };
        navigate(FLOW_CANVAS_DRAFT_ROUTE, { state: draft });
      } catch (err) {
        log('import failed: %o', err);
        setError(t('flows.import.error'));
      }
    },
    [navigate, t]
  );

  /** "New workflow" opens the Phase 4a chooser (scratch / template / describe). */
  const handleNewWorkflow = useCallback(() => {
    log('new workflow: opening chooser');
    setChooserOpen(true);
  }, []);

  /** Create a flow from an empty-state gallery card and open its canvas. */
  const handleEmptyTemplate = useCallback(
    (template: FlowTemplate) => {
      log('empty-state template selected: id=%s', template.id);
      void emptyCreate.create(template.id, t(templateNameKey(template.id)), template.graph);
    },
    [emptyCreate, t]
  );

  const { view, setView, nav } = usePageWelcomeView({
    ariaLabel: t('nav.flows'),
    welcomeLabel: t('flows.welcome.nav'),
    mainLabel: t('flows.welcome.main'),
    mainIconPath:
      'M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2m-6 9l2 2 4-4',
    // Sub-pages within Workflows: aggregate Runs, and Flow Scout Discoveries.
    extraItems: [
      {
        value: 'runs',
        label: t('nav.workflowRuns'),
        iconPath: 'M3 3v5h5M3.05 13A9 9 0 106 5.3L3 8M12 7v5l3 2',
      },
      {
        value: 'discoveries',
        label: t('nav.workflowDiscoveries'),
        iconPath: 'M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z',
      },
    ],
  });

  if (view === 'welcome') {
    return (
      <>
        {nav}
        <PageWelcome
          testId="flows-welcome"
          accent="sage"
          icon="⚡"
          eyebrow={t('flows.welcome.eyebrow')}
          title={t('flows.welcome.title')}
          description={t('flows.welcome.body')}
          ctas={[
            {
              label: t('flows.welcome.ctaNew'),
              icon: '✨',
              onClick: handleNewWorkflow,
              testId: 'flows-welcome-cta-new',
            },
            { label: t('flows.welcome.ctaBrowse'), icon: '📂', onClick: () => setView('main') },
          ]}
          featuresHeading={t('flows.welcome.featsLabel')}
          features={[
            {
              icon: '✍️',
              title: t('flows.welcome.feat1Title'),
              description: t('flows.welcome.feat1Body'),
            },
            {
              icon: '⏱️',
              title: t('flows.welcome.feat2Title'),
              description: t('flows.welcome.feat2Body'),
            },
            {
              icon: '🧑‍⚖️',
              title: t('flows.welcome.feat3Title'),
              description: t('flows.welcome.feat3Body'),
            },
          ]}
        />
        {chooserOpen && <NewWorkflowModal onClose={() => setChooserOpen(false)} />}
      </>
    );
  }

  if (view === 'runs') {
    return (
      <>
        {nav}
        <WorkflowRunsPage />
      </>
    );
  }

  if (view === 'discoveries') {
    return (
      <>
        {nav}
        <WorkflowDiscoveriesPage />
      </>
    );
  }

  return (
    <>
      {nav}
      <PanelPage testId="flows-page" contentClassName="p-4">
        <input
          ref={importInputRef}
          type="file"
          accept="application/json,.json"
          className="hidden"
          data-testid="flows-import-input"
          onChange={e => void handleImportFile(e)}
        />
        <div className="mx-auto w-full max-w-3xl space-y-4">
          <PageSectionHeader
            title={t('flows.page.title')}
            description={t('flows.page.description')}
            action={
              <div className="flex items-center gap-2">
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  data-testid="flows-import"
                  onClick={handleImportClick}>
                  {t('flows.page.import')}
                </Button>
                <Button
                  type="button"
                  variant="primary"
                  size="sm"
                  data-testid="flows-new-workflow"
                  onClick={handleNewWorkflow}>
                  {t('flows.page.newWorkflow')}
                </Button>
              </div>
            }
          />
          <div data-testid="flows-beta-banner">
            <BetaBanner />
          </div>

          {/* Flow Scout discovery moved to its own sidebar page
              (/flows/discoveries); the list stays focused on saved workflows. */}

          {error && (
            <div data-testid="flows-error">
              <ErrorBanner message={error} />
            </div>
          )}

          {loading && <CenteredLoadingState label={t('flows.page.loading')} />}

          {!loading && flows.length === 0 && !error && (
            <div className="space-y-4">
              <EmptyStateCard
                icon={
                  <svg
                    className="h-7 w-7 text-primary-500"
                    fill="none"
                    viewBox="0 0 24 24"
                    stroke="currentColor"
                    strokeWidth={1.5}>
                    <circle cx="5" cy="6" r="2" />
                    <circle cx="5" cy="18" r="2" />
                    <circle cx="19" cy="12" r="2" />
                    <path strokeLinecap="round" d="M7 6h4a4 4 0 014 4M7 18h4a4 4 0 004-4" />
                  </svg>
                }
                title={t('flows.page.emptyTitle')}
                description={t('flows.page.emptyDescription')}
                actionLabel={t('flows.page.newWorkflow')}
                actionTestId="flows-empty-new-workflow"
                onAction={handleNewWorkflow}
              />

              <section className="space-y-3" data-testid="flows-empty-templates">
                <div>
                  <h3 className="text-sm font-semibold text-content">
                    {t('flows.templates.title')}
                  </h3>
                  <p className="text-xs text-content-muted">{t('flows.templates.subtitle')}</p>
                </div>
                {emptyCreate.error && (
                  <div data-testid="flows-empty-template-error">
                    <ErrorBanner message={emptyCreate.error} />
                  </div>
                )}
                <FlowTemplateGallery onSelect={handleEmptyTemplate} busyId={emptyCreate.busyKey} />
              </section>
            </div>
          )}

          {!loading && flows.length > 0 && (
            <div
              data-testid="flows-list"
              className="overflow-hidden rounded-2xl border border-line bg-surface">
              {flows.map(flow => (
                <FlowListRow
                  key={flow.id}
                  flow={flow}
                  busy={busyFor(flow)}
                  onToggle={f => void handleToggle(f)}
                  onRun={f => void handleRun(f)}
                  onViewRuns={handleViewRuns}
                  onView={handleView}
                  onExport={handleExport}
                  onDuplicate={f => void handleDuplicate(f)}
                  onDelete={setDeleteTarget}
                />
              ))}
            </div>
          )}
        </div>

        <FlowRunsDrawer
          flowId={selectedFlowId}
          flowName={selectedFlow?.name}
          onClose={() => setSelectedFlowId(null)}
          onFixWithAgent={handleFixWithAgent}
        />

        {chooserOpen && <NewWorkflowModal onClose={() => setChooserOpen(false)} />}

        {deleteTarget && (
          <ModalShell
            onClose={() => (deleting ? undefined : setDeleteTarget(null))}
            title={t('flows.delete.title')}
            subtitle={t('flows.delete.body').replace('{name}', deleteTarget.name)}
            titleId="flow-delete-modal-title"
            maxWidthClassName="max-w-sm">
            <div className="flex justify-end gap-2" data-testid="flow-delete-confirm">
              <Button
                type="button"
                variant="secondary"
                size="sm"
                disabled={deleting}
                data-testid="flow-delete-cancel"
                onClick={() => setDeleteTarget(null)}>
                {t('flows.delete.cancel')}
              </Button>
              <Button
                type="button"
                variant="primary"
                tone="danger"
                size="sm"
                disabled={deleting}
                data-testid="flow-delete-confirm-button"
                onClick={() => void handleConfirmDelete()}>
                {deleting ? t('flows.delete.deleting') : t('flows.delete.confirm')}
              </Button>
            </div>
          </ModalShell>
        )}

        {preauth.pending && (
          <FlowPreauthorizationOverlay
            entries={preauth.pending.manifest.entries}
            busy={preauth.busy}
            errorMsg={preauth.errorKey ? t('flows.enableApproval.error') : null}
            onApproveAll={() => void preauth.approveAll()}
            onDeny={() => void preauth.deny()}
          />
        )}

        <ToastContainer notifications={toasts} onRemove={removeToast} />
      </PanelPage>
    </>
  );
}
