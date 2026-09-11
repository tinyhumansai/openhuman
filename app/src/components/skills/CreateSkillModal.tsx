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
 * The form fields + submit pipeline live in `CreateWorkflowForm` so the
 * `/skills/new` page can share the exact same body. This file supplies
 * the chrome through the shared `ModalShell` primitive — which owns the
 * backdrop, Escape handling, focus trap and focus-return — plus the
 * submit/cancel footer. The footer's submit button is wired to the form
 * via the standard HTML `form=` attribute so we don't need an imperative
 * handle here.
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { type WorkflowSummary } from '../../services/api/skillsApi';
import { ModalShell } from '../ui';
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

  useEffect(() => {
    log('mount');
    return () => log('unmount');
  }, []);

  const handleStateChange = useCallback((state: { valid: boolean; submitting: boolean }) => {
    setFormValid(state.valid);
    setSubmitting(state.submitting);
  }, []);

  return (
    <ModalShell
      onClose={() => {
        if (submitting) return;
        log('close-request');
        onClose();
      }}
      title={editing ? t('common.edit') : t('workflows.create.title')}
      titleId="create-skill-title"
      subtitle={t('workflows.create.subtitle')}
      maxWidthClassName="max-w-[520px]"
      contentClassName="max-h-[70vh] overflow-y-auto px-5 py-4"
      closePolicy={submitting ? { escape: false, backdrop: false, button: false } : undefined}
      footer={
        <div className="flex items-center justify-end gap-2">
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
      }>
      <CreateWorkflowForm
        formId={CREATE_FORM_ID}
        onCreated={onCreated}
        onStateChange={handleStateChange}
        autoFocus
        editing={editing}
      />
    </ModalShell>
  );
}
