/**
 * /skills/new — full-page Create-a-Skill authoring view.
 *
 * Renders `CreateSkillForm` (extracted from CreateSkillModal in
 * Phase 5) inside page chrome, so the same flow is available as a
 * standalone route — entry point for the Skills dashboard's [+ Create
 * a Skill] CTA and bookmark-able for users who routinely scaffold
 * new SKILL.md drafts.
 *
 * Behaviour on submit:
 *   - Success → navigate to /skills (dashboard) so the user lands
 *     somewhere meaningful. We considered /skills/run?skill=<new-id>,
 *     but new skills aren't auto-scheduled and the runner picker
 *     pre-select only makes sense once the user has filled in inputs.
 *     Dashboard sends a clearer "skill created, here's what's
 *     scheduled" signal.
 *   - Cancel → /skills.
 */
import { useCallback, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import CreateSkillForm from '../components/skills/CreateSkillForm';
import { useT } from '../lib/i18n/I18nContext';
import { type SkillSummary } from '../services/api/skillsApi';

const PAGE_FORM_ID = 'create-skill-page-form';

export default function SkillNew() {
  const { t } = useT();
  const navigate = useNavigate();

  const [formValid, setFormValid] = useState(false);
  const [submitting, setSubmitting] = useState(false);

  const handleStateChange = useCallback((state: { valid: boolean; submitting: boolean }) => {
    setFormValid(state.valid);
    setSubmitting(state.submitting);
  }, []);

  const handleCreated = useCallback(
    (_skill: SkillSummary) => {
      // The dashboard re-fetches the cron list on mount, so any
      // schedule the user adds for this new skill will appear there
      // automatically — no need to plumb the new id through state.
      navigate('/skills?tab=runners');
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
              <h1 className="text-base font-semibold text-stone-900 dark:text-neutral-100">
                {t('skills.new.title')}
              </h1>
              <p className="mt-0.5 text-xs text-stone-500 dark:text-neutral-400">
                {t('skills.create.subtitle')}
              </p>
            </div>
            <div className="flex items-center gap-2 shrink-0">
              <button
                type="button"
                data-testid="skill-new-cancel"
                onClick={() => navigate('/skills?tab=runners')}
                disabled={submitting}
                className="rounded-lg px-4 py-2 text-sm font-medium text-stone-600 dark:text-neutral-300 transition-colors hover:bg-stone-100 dark:hover:bg-neutral-800 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1 disabled:opacity-40">
                {t('common.cancel')}
              </button>
              <button
                type="submit"
                form={PAGE_FORM_ID}
                data-testid="skill-new-submit"
                disabled={!formValid || submitting}
                className="rounded-lg bg-primary-500 px-4 py-2 text-sm font-semibold text-white shadow-soft transition-colors hover:bg-primary-600 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1 disabled:cursor-not-allowed disabled:opacity-50">
                {submitting ? t('skills.create.creating') : t('skills.create.createBtn')}
              </button>
            </div>
          </div>

          {/* Form */}
          <div className="rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-6 shadow-soft">
            <CreateSkillForm
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
