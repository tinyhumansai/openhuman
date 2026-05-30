/**
 * AgentWorkflows page
 * -------------------
 *
 * Lists, creates, views, and deletes agent workflows. Mirrors the Skills page
 * structure: cards for each workflow, a detail drawer on click, a create
 * modal, and delete confirmation. Workflow data is loaded via the Redux
 * `workflows` slice backed by `workflowsApi`.
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { ToastContainer } from '../components/intelligence/Toast';
import CreateWorkflowModal from '../components/workflows/CreateWorkflowModal';
import WorkflowCard from '../components/workflows/WorkflowCard';
import WorkflowDetailDrawer from '../components/workflows/WorkflowDetailDrawer';
import { useT } from '../lib/i18n/I18nContext';
import type { Workflow, WorkflowSummary } from '../services/api/workflowsApi';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import {
  loadWorkflows,
  removeWorkflow,
  selectWorkflows,
  selectWorkflowsError,
  selectWorkflowsStatus,
} from '../store/workflowsSlice';
import type { ToastNotification } from '../types/intelligence';

const log = debug('agentWorkflows');

export default function AgentWorkflows() {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const workflows = useAppSelector(selectWorkflows);
  const status = useAppSelector(selectWorkflowsStatus);
  const storeError = useAppSelector(selectWorkflowsError);

  const [selectedWorkflow, setSelectedWorkflow] = useState<WorkflowSummary | null>(null);
  const [createModalOpen, setCreateModalOpen] = useState(false);
  const [deleteCandidate, setDeleteCandidate] = useState<WorkflowSummary | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [toasts, setToasts] = useState<ToastNotification[]>([]);

  const addToast = useCallback((toast: Omit<ToastNotification, 'id'>) => {
    const newToast: ToastNotification = { ...toast, id: `toast-${Date.now()}-${Math.random()}` };
    setToasts(prev => [...prev, newToast]);
  }, []);

  const removeToast = useCallback((id: string) => {
    setToasts(prev => prev.filter(t => t.id !== id));
  }, []);

  // Initial load of workflows.
  useEffect(() => {
    log('mount — dispatch loadWorkflows');
    void dispatch(loadWorkflows());
  }, [dispatch]);

  const handleCreated = useCallback(
    (workflow: Workflow) => {
      log('created workflow name=%s', workflow.name);
      setCreateModalOpen(false);
      // Refresh list to pick up newly created workflow with full server data.
      void dispatch(loadWorkflows());
      addToast({
        type: 'success',
        title: t('workflows.create.successTitle'),
        message: `"${workflow.name}" ${t('workflows.create.successMessage')}`,
      });
    },
    [dispatch, addToast, t]
  );

  const handleDeleteConfirm = useCallback(async () => {
    if (!deleteCandidate || deleting) return;
    setDeleting(true);
    log('delete workflow id=%s', deleteCandidate.id);
    try {
      await dispatch(removeWorkflow(deleteCandidate.id)).unwrap();
      log('deleted workflow id=%s', deleteCandidate.id);
      // Close the drawer if it was showing the deleted workflow.
      setSelectedWorkflow(prev => (prev && prev.id === deleteCandidate.id ? null : prev));
      addToast({
        type: 'success',
        title: t('workflows.delete'),
        message: `"${deleteCandidate.name}" ${t('common.success')}`,
      });
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('delete error id=%s %s', deleteCandidate.id, msg);
      addToast({ type: 'error', title: t('workflows.deleteError'), message: msg });
    } finally {
      setDeleting(false);
      setDeleteCandidate(null);
    }
  }, [deleteCandidate, deleting, dispatch, addToast, t]);

  const isLoading = status === 'loading';
  const isEmpty = workflows.length === 0 && !isLoading;

  return (
    <div className="min-h-full">
      <div className="min-h-full flex flex-col">
        <div className="flex-1 flex items-start justify-center p-4 pt-6">
          <div className="w-full max-w-3xl space-y-4">
            {/* Header */}
            <div className="flex items-center justify-between gap-2">
              <div className="min-w-0">
                <h1 className="text-base font-semibold text-stone-900 dark:text-neutral-100">
                  {t('workflows.title')}
                </h1>
                <p className="mt-0.5 text-xs text-stone-500 dark:text-neutral-400">
                  {t('workflows.subtitle')}
                </p>
              </div>
              <button
                type="button"
                data-testid="workflows-create-btn"
                onClick={() => setCreateModalOpen(true)}
                className="flex-shrink-0 rounded-lg bg-primary-500 px-3 py-2 text-xs font-semibold text-white shadow-soft transition-colors hover:bg-primary-600 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1">
                {t('workflows.createNew')}
              </button>
            </div>

            {/* Error banner */}
            {storeError ? (
              <div className="rounded-2xl border border-coral-200 bg-coral-50 p-3 shadow-soft">
                <div className="flex items-start justify-between gap-3">
                  <p className="text-xs text-coral-800">{storeError}</p>
                  <button
                    type="button"
                    onClick={() => void dispatch(loadWorkflows())}
                    className="flex-shrink-0 rounded-lg border border-coral-200 bg-white px-3 py-1.5 text-[11px] font-medium text-coral-700 hover:bg-coral-50">
                    {t('common.retry')}
                  </button>
                </div>
              </div>
            ) : null}

            {/* Loading skeleton */}
            {isLoading && workflows.length === 0 ? (
              <div className="space-y-2 animate-pulse">
                {[1, 2, 3].map(i => (
                  <div key={i} className="h-20 rounded-2xl bg-stone-100 dark:bg-neutral-800" />
                ))}
              </div>
            ) : null}

            {/* Empty state */}
            {isEmpty ? (
              <div className="rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-10 text-center shadow-soft animate-fade-up">
                <div className="mx-auto mb-3 flex h-12 w-12 items-center justify-center rounded-full bg-primary-50 dark:bg-primary-500/10">
                  <svg
                    className="h-6 w-6 text-primary-500"
                    fill="none"
                    viewBox="0 0 24 24"
                    stroke="currentColor"
                    strokeWidth={1.5}>
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      d="M3.75 12h16.5m-16.5 3.75h16.5M3.75 19.5h16.5M5.625 4.5h12.75a1.875 1.875 0 010 3.75H5.625a1.875 1.875 0 010-3.75z"
                    />
                  </svg>
                </div>
                <h2 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                  {t('workflows.empty.title')}
                </h2>
                <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
                  {t('workflows.empty.body')}
                </p>
                <button
                  type="button"
                  onClick={() => setCreateModalOpen(true)}
                  className="mt-4 rounded-lg bg-primary-500 px-4 py-2 text-xs font-semibold text-white shadow-soft hover:bg-primary-600 focus:outline-none focus:ring-2 focus:ring-primary-500">
                  {t('workflows.createNew')}
                </button>
              </div>
            ) : null}

            {/* Workflow list */}
            {workflows.length > 0 ? (
              <div
                className="rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-3 shadow-soft animate-fade-up"
                data-testid="workflows-list">
                <div className="px-1 pb-3 pt-1">
                  <h2 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                    {t('workflows.listHeading')} ({workflows.length})
                  </h2>
                </div>
                <div className="space-y-2">
                  {workflows.map(wf => (
                    <WorkflowCard
                      key={wf.id}
                      workflow={wf}
                      testId={`workflow-card-${wf.id}`}
                      onView={w => {
                        log('open drawer workflowId=%s', w.id);
                        setSelectedWorkflow(w);
                      }}
                      onDelete={w => {
                        log('open delete candidate workflowId=%s', w.id);
                        setDeleteCandidate(w);
                      }}
                    />
                  ))}
                </div>
              </div>
            ) : null}
          </div>
        </div>
      </div>

      {/* Detail drawer */}
      {selectedWorkflow ? (
        <WorkflowDetailDrawer
          workflow={selectedWorkflow}
          onClose={() => setSelectedWorkflow(null)}
        />
      ) : null}

      {/* Create modal */}
      {createModalOpen ? (
        <CreateWorkflowModal onClose={() => setCreateModalOpen(false)} onCreated={handleCreated} />
      ) : null}

      {/* Delete confirmation dialog */}
      {deleteCandidate ? (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center p-4"
          onClick={e => {
            if (e.target === e.currentTarget && !deleting) setDeleteCandidate(null);
          }}>
          <div
            aria-hidden="true"
            className="absolute inset-0 bg-black/50 backdrop-blur-sm animate-fade-in"
            onClick={() => {
              if (!deleting) setDeleteCandidate(null);
            }}
          />
          <div
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="wf-delete-title"
            className="relative w-full max-w-sm rounded-2xl bg-white dark:bg-neutral-900 shadow-2xl animate-fade-in p-6 space-y-4">
            <h2
              id="wf-delete-title"
              className="text-base font-semibold text-stone-900 dark:text-neutral-100">
              {t('workflows.deleteConfirm.title')}
            </h2>
            <p className="text-sm text-stone-600 dark:text-neutral-300">
              {t('workflows.deleteConfirm.body').replace('{name}', deleteCandidate.name)}
            </p>
            <div className="flex items-center justify-end gap-2">
              <button
                type="button"
                onClick={() => setDeleteCandidate(null)}
                disabled={deleting}
                className="rounded-lg px-4 py-2 text-sm font-medium text-stone-600 dark:text-neutral-300 transition-colors hover:bg-stone-100 dark:hover:bg-neutral-800 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1 disabled:opacity-40">
                {t('common.cancel')}
              </button>
              <button
                type="button"
                data-testid="wf-delete-confirm-btn"
                onClick={() => void handleDeleteConfirm()}
                disabled={deleting}
                className="rounded-lg bg-coral-600 px-4 py-2 text-sm font-semibold text-white transition-colors hover:bg-coral-700 focus:outline-none focus:ring-2 focus:ring-coral-500 focus:ring-offset-1 disabled:opacity-50 disabled:cursor-not-allowed">
                {deleting ? t('common.loading') : t('common.delete')}
              </button>
            </div>
          </div>
        </div>
      ) : null}

      <ToastContainer notifications={toasts} onRemove={removeToast} />
    </div>
  );
}
