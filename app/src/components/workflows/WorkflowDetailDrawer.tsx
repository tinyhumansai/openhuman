/**
 * WorkflowDetailDrawer
 * --------------------
 *
 * Right-side slide-in drawer surfacing full metadata for a workflow plus its
 * phases. Mirrors SkillDetailDrawer — rendered via createPortal, Escape/
 * backdrop click to close, focus capture on open.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

import { useT } from '../../lib/i18n/I18nContext';
import { type Workflow, workflowsApi, type WorkflowSummary } from '../../services/api/workflowsApi';
import PhaseEditor from './PhaseEditor';

const log = debug('workflows:drawer');

interface Props {
  workflow: WorkflowSummary;
  onClose: () => void;
}

function scopePillCls(scope: WorkflowSummary['scope']): string {
  switch (scope) {
    case 'user':
      return 'bg-sage-50 text-sage-700 border-sage-200';
    case 'project':
      return 'bg-amber-50 text-amber-700 border-amber-200';
    default:
      return 'bg-stone-100 dark:bg-neutral-800 text-stone-700 dark:text-neutral-200 border-stone-200 dark:border-neutral-700';
  }
}

export default function WorkflowDetailDrawer({ workflow, onClose }: Props) {
  const { t } = useT();
  const closeBtnRef = useRef<HTMLButtonElement | null>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);
  const [fullWorkflow, setFullWorkflow] = useState<Workflow | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  // Capture focus on mount, restore on unmount.
  useEffect(() => {
    previousFocusRef.current = document.activeElement as HTMLElement | null;
    const raf = window.requestAnimationFrame(() => {
      closeBtnRef.current?.focus();
    });
    log('mount workflowId=%s', workflow.id);
    return () => {
      window.cancelAnimationFrame(raf);
      previousFocusRef.current?.focus?.();
      log('unmount workflowId=%s', workflow.id);
    };
  }, [workflow.id]);

  // Close on Escape key.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        log('escape-key close workflowId=%s', workflow.id);
        onClose();
      }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [onClose, workflow.id]);

  // Load full workflow on mount.
  useEffect(() => {
    let cancelled = false;
    setFullWorkflow(null);
    setLoadError(null);
    void workflowsApi
      .readWorkflow(workflow.id)
      .then(wf => {
        if (!cancelled) {
          log('loaded full workflow id=%s phases=%d', wf.dir_name, Object.keys(wf.phases).length);
          setFullWorkflow(wf);
        }
      })
      .catch(err => {
        if (!cancelled) {
          const msg = err instanceof Error ? err.message : String(err);
          log('load error id=%s %s', workflow.id, msg);
          setLoadError(msg);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [workflow.id]);

  const handleBackdropClick = useCallback(() => {
    log('backdrop close workflowId=%s', workflow.id);
    onClose();
  }, [onClose, workflow.id]);

  const pillCls = scopePillCls(workflow.scope);
  const scopeLabel = workflow.scope === 'user' ? t('scope.user') : t('scope.project');

  const phases = fullWorkflow ? Object.entries(fullWorkflow.phases) : [];

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex"
      onClick={e => {
        if (e.target === e.currentTarget) handleBackdropClick();
      }}>
      {/* Backdrop */}
      <div
        aria-hidden="true"
        className="absolute inset-0 bg-black/50 backdrop-blur-sm animate-fade-in"
        onClick={handleBackdropClick}
      />

      {/* Drawer */}
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="workflow-drawer-title"
        className="relative ml-auto flex h-full w-full max-w-[540px] flex-col bg-white dark:bg-neutral-900 shadow-2xl animate-slide-in-right">
        {/* Header */}
        <div className="flex items-start justify-between gap-3 border-b border-stone-100 dark:border-neutral-800 px-5 py-4">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <h2
                id="workflow-drawer-title"
                className="truncate text-base font-semibold text-stone-900 dark:text-neutral-100 font-sans">
                {workflow.name}
              </h2>
              <span
                className={`inline-flex flex-shrink-0 items-center rounded-full border px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide ${pillCls}`}>
                {scopeLabel}
              </span>
            </div>
            {workflow.when_to_use ? (
              <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400 italic">
                {workflow.when_to_use}
              </p>
            ) : null}
          </div>
          <button
            ref={closeBtnRef}
            type="button"
            onClick={() => {
              log('close-button workflowId=%s', workflow.id);
              onClose();
            }}
            aria-label={t('common.close')}
            className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-lg text-stone-400 dark:text-neutral-500 transition-colors hover:bg-stone-100 dark:hover:bg-neutral-800 hover:text-stone-600 dark:hover:text-neutral-300 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1">
            <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M6 18L18 6M6 6l12 12"
              />
            </svg>
          </button>
        </div>

        {/* Body */}
        <div className="flex-1 overflow-y-auto">
          <div className="px-5 py-4 space-y-4">
            {/* Description */}
            {workflow.description ? (
              <p className="text-sm leading-relaxed text-stone-700 dark:text-neutral-200 font-sans">
                {workflow.description}
              </p>
            ) : null}

            {/* Metadata */}
            <dl className="grid grid-cols-[auto,1fr] gap-x-3 gap-y-2 text-xs">
              {workflow.tags.length > 0 ? (
                <>
                  <dt className="font-medium text-stone-500 dark:text-neutral-400">
                    {t('workflows.detail.tags')}
                  </dt>
                  <dd className="flex flex-wrap gap-1">
                    {workflow.tags.map(tag => (
                      <span
                        key={tag}
                        className="inline-flex items-center rounded-md border border-stone-200 dark:border-neutral-700 bg-stone-50 dark:bg-neutral-800/60 px-1.5 py-0.5 text-[10px] text-stone-700 dark:text-neutral-200">
                        {tag}
                      </span>
                    ))}
                  </dd>
                </>
              ) : null}
              {fullWorkflow?.location ? (
                <>
                  <dt className="font-medium text-stone-500 dark:text-neutral-400">
                    {t('workflows.detail.location')}
                  </dt>
                  <dd
                    className="truncate font-mono text-[11px] text-stone-600 dark:text-neutral-300"
                    title={fullWorkflow.location}>
                    {fullWorkflow.location}
                  </dd>
                </>
              ) : null}
            </dl>

            {/* Warnings */}
            {workflow.warnings.length > 0 ? (
              <div className="rounded-xl border border-amber-200 bg-amber-50 p-3">
                <p className="text-[11px] font-semibold uppercase tracking-wide text-amber-900">
                  {t('workflows.detail.warnings')}
                </p>
                <ul className="mt-1.5 list-disc space-y-1 pl-4 text-xs text-amber-800">
                  {workflow.warnings.map((w, i) => (
                    <li key={i}>{w}</li>
                  ))}
                </ul>
              </div>
            ) : null}

            {/* Phases */}
            <div>
              <h3 className="mb-3 text-[11px] font-semibold uppercase tracking-wide text-stone-500 dark:text-neutral-400">
                {t('workflows.detail.phases')} ({workflow.phases.length})
              </h3>
              {loadError ? (
                <div className="rounded-xl border border-coral-200 bg-coral-50 p-3 text-xs text-coral-800">
                  {t('workflows.detail.loadError')}: {loadError}
                </div>
              ) : fullWorkflow === null ? (
                <p className="text-xs text-stone-400 dark:text-neutral-500 animate-pulse">
                  {t('common.loading')}
                </p>
              ) : phases.length === 0 ? (
                <p className="text-xs italic text-stone-400 dark:text-neutral-500">
                  {t('workflows.detail.noPhases')}
                </p>
              ) : (
                <div className="space-y-3">
                  {phases.map(([phaseName, phase]) => (
                    <PhaseEditor key={phaseName} phaseName={phaseName} phase={phase} />
                  ))}
                </div>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>,
    document.body
  );
}
