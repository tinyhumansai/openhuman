/**
 * CreateWorkflowModal
 * -------------------
 *
 * Centered modal that scaffolds a new workflow via `openhuman.workflows_create`.
 * Mirrors CreateSkillModal — backdrop, Escape-to-close, focus capture, 520px
 * max-width, clean white background.
 */
import debug from 'debug';
import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

import { useT } from '../../lib/i18n/I18nContext';
import { type Workflow, workflowsApi } from '../../services/api/workflowsApi';

const log = debug('workflows:create-modal');

interface Props {
  onClose: () => void;
  onCreated: (workflow: Workflow) => void;
}

export default function CreateWorkflowModal({ onClose, onCreated }: Props) {
  const { t } = useT();
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [whenToUse, setWhenToUse] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const nameInputRef = useRef<HTMLInputElement | null>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);

  const isValid = name.trim().length > 0;

  useEffect(() => {
    previousFocusRef.current = document.activeElement as HTMLElement | null;
    const raf = window.requestAnimationFrame(() => {
      nameInputRef.current?.focus();
    });
    log('mount');
    return () => {
      window.cancelAnimationFrame(raf);
      previousFocusRef.current?.focus?.();
      log('unmount');
    };
  }, []);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !submitting) {
        log('escape-key close');
        onClose();
      }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [onClose, submitting]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!isValid || submitting) return;
    setSubmitting(true);
    setError(null);
    log('submit name=%s', name.trim());
    try {
      const workflow = await workflowsApi.createWorkflow({
        name: name.trim(),
        description: description.trim() || undefined,
        when_to_use: whenToUse.trim() || undefined,
      });
      log('created name=%s', workflow.name);
      onCreated(workflow);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('create error %s', msg);
      setError(msg);
      setSubmitting(false);
    }
  };

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4"
      onClick={e => {
        if (e.target === e.currentTarget && !submitting) {
          log('backdrop-click close');
          onClose();
        }
      }}>
      <div
        aria-hidden="true"
        className="absolute inset-0 bg-black/50 backdrop-blur-sm animate-fade-in"
        onClick={() => {
          if (!submitting) {
            log('backdrop-direct close');
            onClose();
          }
        }}
      />

      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="create-workflow-title"
        className="relative w-full max-w-[520px] rounded-2xl bg-white dark:bg-neutral-900 shadow-2xl animate-fade-in">
        {/* Header */}
        <div className="flex items-start justify-between gap-3 border-b border-stone-100 dark:border-neutral-800 px-5 py-4">
          <div className="min-w-0 flex-1">
            <h2
              id="create-workflow-title"
              className="text-base font-semibold text-stone-900 dark:text-neutral-100 font-sans">
              {t('workflows.create.title')}
            </h2>
            <p className="mt-0.5 text-xs text-stone-500 dark:text-neutral-400">
              {t('workflows.create.subtitle')}
            </p>
          </div>
          <button
            type="button"
            onClick={() => {
              if (!submitting) {
                log('close-button');
                onClose();
              }
            }}
            disabled={submitting}
            aria-label={t('common.close')}
            className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-lg text-stone-400 dark:text-neutral-500 transition-colors hover:bg-stone-100 dark:hover:bg-neutral-800 hover:text-stone-600 dark:hover:text-neutral-300 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1 disabled:opacity-40">
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
        <form id="create-workflow-form" onSubmit={e => void handleSubmit(e)}>
          <div className="max-h-[65vh] overflow-y-auto px-5 py-4 space-y-4">
            {/* Name */}
            <div>
              <label
                htmlFor="wf-name"
                className="block text-xs font-medium text-stone-700 dark:text-neutral-200 mb-1">
                {t('workflows.create.name')}
                <span className="ml-1 text-coral-500">*</span>
              </label>
              <input
                ref={nameInputRef}
                id="wf-name"
                type="text"
                value={name}
                onChange={e => setName(e.target.value)}
                disabled={submitting}
                placeholder={t('workflows.create.namePlaceholder')}
                required
                className="w-full rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder-stone-400 dark:placeholder-neutral-500 focus:border-primary-400 focus:outline-none focus:ring-2 focus:ring-primary-500/20 disabled:opacity-50"
              />
            </div>

            {/* Description */}
            <div>
              <label
                htmlFor="wf-description"
                className="block text-xs font-medium text-stone-700 dark:text-neutral-200 mb-1">
                {t('workflows.create.description')}
                <span className="ml-1 text-stone-400 dark:text-neutral-500 font-normal">
                  {t('workflows.create.optional')}
                </span>
              </label>
              <textarea
                id="wf-description"
                value={description}
                onChange={e => setDescription(e.target.value)}
                disabled={submitting}
                placeholder={t('workflows.create.descriptionPlaceholder')}
                rows={2}
                className="w-full resize-none rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder-stone-400 dark:placeholder-neutral-500 focus:border-primary-400 focus:outline-none focus:ring-2 focus:ring-primary-500/20 disabled:opacity-50"
              />
            </div>

            {/* When to use */}
            <div>
              <label
                htmlFor="wf-when-to-use"
                className="block text-xs font-medium text-stone-700 dark:text-neutral-200 mb-1">
                {t('workflows.create.whenToUse')}
                <span className="ml-1 text-stone-400 dark:text-neutral-500 font-normal">
                  {t('workflows.create.optional')}
                </span>
              </label>
              <textarea
                id="wf-when-to-use"
                value={whenToUse}
                onChange={e => setWhenToUse(e.target.value)}
                disabled={submitting}
                placeholder={t('workflows.create.whenToUsePlaceholder')}
                rows={2}
                className="w-full resize-none rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder-stone-400 dark:placeholder-neutral-500 focus:border-primary-400 focus:outline-none focus:ring-2 focus:ring-primary-500/20 disabled:opacity-50"
              />
            </div>

            {/* Error */}
            {error ? (
              <div className="rounded-xl border border-coral-200 bg-coral-50 p-3 text-xs text-coral-800">
                {t('workflows.create.createError')}: {error}
              </div>
            ) : null}
          </div>
        </form>

        {/* Footer */}
        <div className="flex items-center justify-end gap-2 border-t border-stone-100 dark:border-neutral-800 px-5 py-3">
          <button
            type="button"
            onClick={onClose}
            disabled={submitting}
            className="rounded-lg px-4 py-2 text-sm font-medium text-stone-600 dark:text-neutral-300 transition-colors hover:bg-stone-100 dark:hover:bg-neutral-800 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1 disabled:opacity-40">
            {t('common.cancel')}
          </button>
          <button
            type="submit"
            form="create-workflow-form"
            disabled={!isValid || submitting}
            className="rounded-lg bg-primary-500 px-4 py-2 text-sm font-semibold text-white transition-colors hover:bg-primary-600 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1 disabled:cursor-not-allowed disabled:opacity-50">
            {submitting ? t('workflows.create.creating') : t('workflows.create.createBtn')}
          </button>
        </div>
      </div>
    </div>,
    document.body
  );
}
