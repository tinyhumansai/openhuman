/**
 * CreateSkillModal
 * ----------------
 *
 * Centered white modal that scaffolds a new SKILL.md skill via the
 * `openhuman.skills_create` JSON-RPC method. Matches the settings-modal
 * design rules (clean white, 520px desktop, 16px radius, backdrop + blur,
 * Escape/click-out to close, focus capture) — see
 * `.claude/rules/15-settings-modal-system.md`.
 *
 * The form fields + submit pipeline live in `CreateSkillForm` so the
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
import { type SkillSummary } from '../../services/api/skillsApi';
import CreateSkillForm from './CreateSkillForm';

const log = debug('skills:create-modal');

const CREATE_FORM_ID = 'create-skill-modal-form';

interface Props {
  onClose: () => void;
  onCreated: (skill: SkillSummary) => void;
}

export default function CreateSkillModal({ onClose, onCreated }: Props) {
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

  const handleStateChange = useCallback(
    (state: { valid: boolean; submitting: boolean }) => {
      setFormValid(state.valid);
      setSubmitting(state.submitting);
    },
    []
  );

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4"
      onClick={(e) => {
        if (e.target === e.currentTarget && !submitting) {
          log('backdrop-click close');
          onClose();
        }
      }}
    >
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
        className="relative w-full max-w-[520px] rounded-2xl bg-white dark:bg-neutral-900 shadow-2xl animate-fade-in"
      >
        {/* Header */}
        <div className="flex items-start justify-between gap-3 border-b border-stone-100 dark:border-neutral-800 px-5 py-4">
          <div className="min-w-0 flex-1">
            <h2
              id="create-skill-title"
              className="text-base font-semibold text-stone-900 dark:text-neutral-100 font-sans"
            >
              {t('skills.create.title')}
            </h2>
            <p className="mt-0.5 text-xs text-stone-500 dark:text-neutral-400">
              {t('skills.create.subtitle')}
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
            className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-lg text-stone-400 dark:text-neutral-500 transition-colors hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800 hover:text-stone-600 dark:hover:text-neutral-300 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1 disabled:opacity-40"
          >
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

        {/* Body — shared form component */}
        <div className="max-h-[70vh] overflow-y-auto px-5 py-4">
          <CreateSkillForm
            formId={CREATE_FORM_ID}
            onCreated={onCreated}
            onStateChange={handleStateChange}
            autoFocus
          />
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-2 border-t border-stone-100 dark:border-neutral-800 px-5 py-3">
          <button
            type="button"
            onClick={onClose}
            disabled={submitting}
            className="rounded-lg px-4 py-2 text-sm font-medium text-stone-600 dark:text-neutral-300 transition-colors hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1 disabled:opacity-40"
          >
            {t('common.cancel')}
          </button>
          <button
            type="submit"
            form={CREATE_FORM_ID}
            disabled={!formValid || submitting}
            className="rounded-lg bg-primary-500 px-4 py-2 text-sm font-semibold text-white transition-colors hover:bg-primary-600 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {submitting ? t('skills.create.creating') : t('skills.create.createBtn')}
          </button>
        </div>
      </div>
    </div>,
    document.body
  );
}
