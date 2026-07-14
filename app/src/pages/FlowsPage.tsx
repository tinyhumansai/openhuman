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
import type { FlowRepairRequest } from '../components/flows/FlowRunInspectorDrawer';
import FlowRunsDrawer from '../components/flows/FlowRunsDrawer';
import FlowTemplateGallery from '../components/flows/FlowTemplateGallery';
import NewWorkflowModal from '../components/flows/NewWorkflowModal';
import SuggestedWorkflows from '../components/flows/SuggestedWorkflows';
import { useCreateFlow } from '../components/flows/useCreateFlow';
import WorkflowPromptBar from '../components/flows/WorkflowPromptBar';
import { ToastContainer } from '../components/intelligence/Toast';
import PageSectionHeader from '../components/layout/PageSectionHeader';
import PageWelcome from '../components/layout/PageWelcome';
import PanelPage from '../components/layout/PanelPage';
import { usePageWelcomeView } from '../components/layout/usePageWelcomeView';
import BetaBanner from '../components/ui/BetaBanner';
import Button from '../components/ui/Button';
import { CenteredLoadingState, ErrorBanner } from '../components/ui/LoadingState';
import { ModalShell } from '../components/ui/ModalShell';
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
  runFlow,
  setFlowEnabled,
} from '../services/api/flowsApi';
import type { ToastNotification } from '../types/intelligence';

const log = createDebug('app:flows');

/** Which single row + action currently has a request in flight, if any. */
type BusyKey = `toggle:${string}` | `run:${string}`;

function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export default function FlowsPage() {
  const { t } = useT();
  const navigate = useNavigate();
  const [flows, setFlows] = useState<Flow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busyKey, setBusyKey] = useState<BusyKey | null>(null);
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

  const handleToggle = useCallback(
    async (flow: Flow) => {
      if (busyKey) return;
      const key: BusyKey = `toggle:${flow.id}`;
      setBusyKey(key);
      setError(null);
      log('toggle: id=%s next=%s', flow.id, !flow.enabled);
      try {
        const updated = await setFlowEnabled(flow.id, !flow.enabled);
        setFlows(prev => prev.map(f => (f.id === updated.id ? updated : f)));
      } catch (err) {
        log('toggle failed: id=%s err=%o', flow.id, err);
        setError(errorMessage(err));
      } finally {
        setBusyKey(null);
      }
    },
    [busyKey]
  );

  const handleRun = useCallback(
    async (flow: Flow) => {
      if (busyKey) return;
      const key: BusyKey = `run:${flow.id}`;
      setBusyKey(key);
      setError(null);
      log('run: id=%s', flow.id);
      try {
        // Fire-and-forget: the caller doesn't wait for the run to finish,
        // just that it kicked off. The refetch below picks up the refreshed
        // `last_run_at` / `last_status` once the engine settles (or, for a
        // still-running flow, on the next manual refresh). Only refetch on
        // success — `loadFlows()` clears `error`, which would otherwise wipe
        // the failure banner set in the `catch` below.
        await runFlow(flow.id);
        addToast({ type: 'success', title: t('flows.list.runStarted') });
        await loadFlows();
      } catch (err) {
        log('run failed: id=%s err=%o', flow.id, err);
        setError(errorMessage(err));
      } finally {
        setBusyKey(null);
      }
    },
    [busyKey, addToast, loadFlows, t]
  );

  const busyFor = (flow: Flow): FlowListRowBusy => {
    if (busyKey === `toggle:${flow.id}`) return 'toggle';
    if (busyKey === `run:${flow.id}`) return 'run';
    return null;
  };

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

          {/* Prompt-first authoring (Phase 5c): describe a workflow and let the
            builder agent propose it. Hero presentation when the list is empty,
            compact otherwise. Always visible, so it's the single "describe a
            workflow" entry point (the chooser modal no longer duplicates it). */}
          <WorkflowPromptBar variant={!loading && flows.length === 0 ? 'hero' : 'compact'} />

          {/* Flow Scout discovery: proactive, buildable workflow suggestions
            grounded in how the user works. Read-only until they click "Build
            this" (→ workflow_builder) and save. */}
          <SuggestedWorkflows />

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

        <ToastContainer notifications={toasts} onRemove={removeToast} />
      </PanelPage>
    </>
  );
}
