/**
 * /skills/run — single-purpose runner page.
 *
 * The Connections → Runners sub-tab (inside Skills.tsx) now shows a
 * scheduled-skills *dashboard* (cards with enable/disable toggles plus
 * Create / Run CTAs). When the user clicks "▷ Run a Skill" from there,
 * or a card-click that wants to take them into the picker for a specific
 * skill, they land HERE — a focused page that just hosts the
 * SkillsRunnerBody picker + form + run-now + save-schedule flow, without
 * the 4-tab Skills.tsx chrome.
 *
 * Bookmark-friendly and shareable via `?skill=<id>` (the body reads the
 * query param and pre-selects the skill — see SkillsRunnerBody.tsx).
 */
import { useNavigate } from 'react-router-dom';

import SkillsRunnerBody from '../components/skills/SkillsRunnerBody';
import { useT } from '../lib/i18n/I18nContext';

export default function SkillsRun() {
  const { t } = useT();
  const navigate = useNavigate();

  return (
    <div className="min-h-full flex flex-col">
      <div className="flex-1 flex items-start justify-center p-4 pt-6">
        <div className="w-full max-w-3xl space-y-4">
          {/* Page header with a "back to dashboard" affordance so the
              user can always retreat to the Runners overview without
              clicking the bottom-tab. Reuses existing common.back +
              skills.tabs.runners keys to avoid an i18n parity churn for
              this single page. */}
          <div className="flex items-center gap-3">
            <button
              type="button"
              onClick={() => navigate('/skills?tab=runners')}
              aria-label={t('common.back')}
              className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium text-stone-600 dark:text-neutral-300 hover:bg-stone-100 dark:hover:bg-neutral-800 transition-colors">
              <span aria-hidden="true">←</span> {t('common.back')}
            </button>
            <h1 className="text-base font-semibold text-stone-900 dark:text-neutral-100">
              {t('skills.tabs.runners')}
            </h1>
          </div>

          <div className="rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-6 shadow-soft animate-fade-up">
            <SkillsRunnerBody />
          </div>
        </div>
      </div>
    </div>
  );
}
