/**
 * CreateSkillModal
 * ----------------
 *
 * Centered white modal that scaffolds a new SKILL.md skill via the
 * `openhuman.workflows_create` JSON-RPC method. Matches the settings-modal
 * design rules (clean white, 520px desktop, 16px radius, backdrop + blur,
 * Escape/click-out to close, focus capture) — see
 * `.claude/rules/15-settings-modal-system.md`.
 *
 * The form fields + submit pipeline live in `CreateWorkflowForm` so the
 * `/skills/new` page can share the exact same body. This file is the
 * modal chrome: header, close-button, backdrop, Escape handler,
 * focus-return, submit/cancel footer. The footer's submit button is
 * wired to the form via the standard HTML `form=` attribute so we
 * don't need an imperative handle here.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

import { useT } from '../../lib/i18n/I18nContext';
import { type WorkflowSummary } from '../../services/api/workflowsApi';
import Button from '../ui/Button';
import CreateWorkflowForm from './CreateWorkflowForm';

const log = debug('skills:create-modal');

const CREATE_FORM_ID = 'create-skill-modal-form';

interface Props {
  onClose: () => void;
  onCreated: (skill: WorkflowSummary) => void;
  /** When set, the modal edits this workflow instead of creating a new one. */
  editing?: WorkflowSummary;
}

export default function CreateSkillModal({ onClose, onCreated, editing }: Props) {
  const { t } = useT();
  const [formValid, setFormValid] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const previousFocusRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    previousFocusRef.current = document.activeElement as HTMLElement | null;
    log('mount');
    return () => {
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

  const handleStateChange = useCallback((state: { valid: boolean; submitting: boolean }) => {
    setFormValid(state.valid);
    setSubmitting(state.submitting);
  }, []);

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
        aria-labelledby="create-skill-title"
        className="relative w-full max-w-[520px] rounded-2xl bg-surface shadow-2xl animate-fade-in">
        {/* Header */}
        <div className="flex items-start justify-between gap-3 border-b border-line-subtle px-5 py-4">
          <div className="min-w-0 flex-1">
            <h2
              id="create-skill-title"
              className="text-base font-semibold text-content font-sans">
              {editing ? t('common.edit') : t('workflows.create.title')}
            </h2>
            <p className="mt-0.5 text-xs text-content-muted">
              {t('workflows.create.subtitle')}
            </p>
          </div>
          <Button
            iconOnly
            variant="tertiary"
            size="md"
            onClick={() => {
              if (!submitting) {
                log('close-button');
                onClose();
              }
            }}
            disabled={submitting}
            aria-label={t('common.close')}
            className="h-8 w-8 flex-shrink-0 text-content-faint">
            <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M6 18L18 6M6 6l12 12"
              />
            </svg>
          </Button>
        </div>

        {/* Body — shared form component */}
        <div className="max-h-[70vh] overflow-y-auto px-5 py-4">
          <CreateWorkflowForm
            formId={CREATE_FORM_ID}
            onCreated={onCreated}
            onStateChange={handleStateChange}
            autoFocus
            editing={editing}
          />
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-2 border-t border-line-subtle px-5 py-3">
          <Button variant="tertiary" onClick={onClose} disabled={submitting}>
            {t('common.cancel')}
          </Button>
          <Button
            type="submit"
            variant="primary"
            form={CREATE_FORM_ID}
            disabled={!formValid || submitting}>
            {submitting
              ? t('workflows.create.creating')
              : editing
                ? t('common.save')
                : t('workflows.create.createBtn')}
          </Button>
        </div>
      </div>
    </div>,
    document.body
  );
}
