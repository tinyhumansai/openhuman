/**
 * /workflows/run — single-purpose workflow runner page.
 *
 * Reached by clicking a workflow card (which locks the page to that
 * workflow via `?workflow=<id>&lock=1`) or any `?workflow=<id>` deep link.
 * Hosts the WorkflowRunnerBody picker + form + run-now + edit + schedule +
 * recent-runs flow without the Connections-page tab chrome.
 *
 * Bookmark-friendly and shareable via `?workflow=<id>` (the body reads the
 * query param and pre-selects the workflow — see WorkflowRunnerBody.tsx).
 */
import { useNavigate } from 'react-router-dom';

import WorkflowRunnerBody from '../components/skills/WorkflowRunnerBody';
import { useT } from '../lib/i18n/I18nContext';

export default function WorkflowsRun() {
  const { t } = useT();
  const navigate = useNavigate();

  return (
    <div className="min-h-full flex flex-col">
      <div className="flex-1 flex items-start justify-center p-4 pt-6">
        <div className="w-full max-w-3xl space-y-4">
          {/* Back goes one layer up to wherever the runner was opened from
              (e.g. the Workflows tab via a workflow's Run/Schedule action),
              not a hard-coded route. Param syncing uses `replace`, so history
              isn't polluted and `-1` lands on the originating page. Falls back
              to the Workflows tab on a cold deep-link with no history. */}
          <div className="flex items-center gap-3">
            <button
              type="button"
              onClick={() =>
                // Use the router history index, not `history.length`: length > 1
                // is true even when the only prior entry is an external referrer,
                // which would bounce the user out of the app. `state.idx > 0`
                // means there's an in-app entry to go back to (matches
                // useSettingsNavigation's guard).
                (window.history.state?.idx ?? 0) > 0
                  ? navigate(-1)
                  : navigate('/intelligence?tab=workflows')
              }
              aria-label={t('common.back')}
              className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium text-stone-600 dark:text-neutral-300 hover:bg-stone-100 dark:hover:bg-neutral-800 transition-colors">
              <span aria-hidden="true">←</span> {t('common.back')}
            </button>
            <h1 className="text-base font-semibold text-stone-900 dark:text-neutral-100">
              {t('skills.run.title')}
            </h1>
          </div>

          <div className="rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-6 shadow-soft animate-fade-up">
            <WorkflowRunnerBody />
          </div>
        </div>
      </div>
    </div>
  );
}
