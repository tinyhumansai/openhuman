/**
 * /workflows/new — full-page Create-a-Skill authoring view.
 *
 * Renders `CreateWorkflowForm` (extracted from CreateSkillModal in
 * Phase 5) inside page chrome, so the same flow is available as a
 * standalone route — entry point for the Skills dashboard's [+ Create
 * a Skill] CTA and bookmark-able for users who routinely scaffold
 * new SKILL.md drafts.
 *
 * Behaviour on submit:
 *   - Success → navigate to /connections so the user lands somewhere
 *     meaningful. We considered /workflows/run?workflow=<new-id>, but
 *     new skills aren't auto-scheduled and the runner picker pre-select
 *     only makes sense once the user has filled in inputs. The
 *     Connections page (defaulting to the Composio tab) provides a clear "here
 *     are your connections" signal. Use ?tab=skills to deep-link to
 *     the Skills tab if needed.
 *   - Cancel → /connections.
 */
import { useCallback, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import CreateWorkflowForm from '../components/skills/CreateWorkflowForm';
import Button from '../components/ui/Button';
import { useT } from '../lib/i18n/I18nContext';
import { type WorkflowSummary } from '../services/api/workflowsApi';

const PAGE_FORM_ID = 'create-skill-page-form';

export default function WorkflowNew() {
  const { t } = useT();
  const navigate = useNavigate();

  const [formValid, setFormValid] = useState(false);
  const [submitting, setSubmitting] = useState(false);

  const handleStateChange = useCallback((state: { valid: boolean; submitting: boolean }) => {
    setFormValid(state.valid);
    setSubmitting(state.submitting);
  }, []);

  const handleCreated = useCallback(
    (_skill: WorkflowSummary) => {
      // The dashboard re-fetches the cron list on mount, so any
      // schedule the user adds for this new skill will appear there
      // automatically — no need to plumb the new id through state.
      navigate('/connections');
    },
    [navigate]
  );

  return (
    <div className="min-h-full flex flex-col">
      <div className="flex-1 flex items-start justify-center p-4 pt-6">
        <div className="w-full max-w-3xl space-y-4">
          {/* Header: title + Cancel/Submit on the right.
              The submit button is wired to the form via `form=PAGE_FORM_ID`
              so it submits the underlying form even though it sits in the
              header rather than inside the form element. */}
          <div className="flex items-center justify-between gap-2">
            <div className="min-w-0">
              <h1 className="text-base font-semibold text-content">{t('skills.new.title')}</h1>
              <p className="mt-0.5 text-xs text-content-muted">{t('skills.create.subtitle')}</p>
            </div>
            <div className="flex items-center gap-2 shrink-0">
              <Button
                variant="tertiary"
                data-testid="skill-new-cancel"
                onClick={() => navigate('/connections')}
                disabled={submitting}>
                {t('common.cancel')}
              </Button>
              <Button
                variant="primary"
                type="submit"
                form={PAGE_FORM_ID}
                data-testid="skill-new-submit"
                disabled={!formValid || submitting}>
                {submitting ? t('skills.create.creating') : t('skills.create.createBtn')}
              </Button>
            </div>
          </div>

          {/* Form */}
          <div className="rounded-2xl border border-line bg-surface p-6 shadow-soft">
            <CreateWorkflowForm
              formId={PAGE_FORM_ID}
              onCreated={handleCreated}
              onStateChange={handleStateChange}
              autoFocus
            />
          </div>
        </div>
      </div>
    </div>
  );
}
