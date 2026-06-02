/**
 * WorkflowCard
 * ------------
 * A list-row card for a single workflow summary. Mirrors the SkillCard pattern.
 * Displays the workflow name, description, scope pill, phase count, tags, and
 * optional action buttons.
 */
import { type FC } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { WorkflowSummary } from '../../services/api/workflowsApi';

interface WorkflowCardProps {
  workflow: WorkflowSummary;
  onView: (workflow: WorkflowSummary) => void;
  onDelete?: (workflow: WorkflowSummary) => void;
  testId?: string;
}

function scopePillCls(scope: WorkflowSummary['scope']): string {
  switch (scope) {
    case 'user':
      return 'bg-sage-50 text-sage-700 border-sage-200';
    case 'project':
      return 'bg-amber-50 text-amber-700 border-amber-200';
    default:
      return 'bg-stone-100 text-stone-700 border-stone-200 dark:bg-neutral-800 dark:text-neutral-200 dark:border-neutral-700';
  }
}

const WorkflowCard: FC<WorkflowCardProps> = ({ workflow, onView, onDelete, testId }) => {
  const { t } = useT();
  const pillCls = scopePillCls(workflow.scope);
  const scopeLabel = workflow.scope === 'user' ? t('scope.user') : t('scope.project');
  const canDelete = workflow.scope === 'user';

  return (
    <div
      data-testid={testId}
      className="flex items-start justify-between gap-3 rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-4 shadow-soft transition-colors hover:bg-stone-50 dark:hover:bg-neutral-800/50">
      {/* Left: content */}
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <h3 className="truncate text-sm font-semibold text-stone-900 dark:text-neutral-100">
            {workflow.name}
          </h3>
          <span
            className={`inline-flex flex-shrink-0 items-center rounded-full border px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide ${pillCls}`}>
            {scopeLabel}
          </span>
        </div>

        {workflow.description ? (
          <p className="mt-1 line-clamp-2 text-xs leading-relaxed text-stone-500 dark:text-neutral-400">
            {workflow.description}
          </p>
        ) : null}

        {/* Phases */}
        {workflow.phases.length > 0 ? (
          <div className="mt-2 flex flex-wrap gap-1">
            {workflow.phases.map(phase => (
              <span
                key={phase}
                className="inline-flex items-center rounded-md border border-primary-100 bg-primary-50 dark:bg-primary-500/10 dark:border-primary-500/30 px-1.5 py-0.5 font-mono text-[10px] text-primary-700 dark:text-primary-300">
                {phase}
              </span>
            ))}
          </div>
        ) : null}

        {/* Tags */}
        {workflow.tags.length > 0 ? (
          <div className="mt-1.5 flex flex-wrap gap-1">
            {workflow.tags.map(tag => (
              <span
                key={tag}
                className="inline-flex items-center rounded-md border border-stone-200 dark:border-neutral-700 bg-stone-50 dark:bg-neutral-800/60 px-1.5 py-0.5 text-[10px] text-stone-600 dark:text-neutral-300">
                {tag}
              </span>
            ))}
          </div>
        ) : null}

        {/* Warnings */}
        {workflow.warnings.length > 0 ? (
          <p className="mt-1.5 text-[11px] text-amber-700 dark:text-amber-300">
            {t('workflows.warnings').replace('{count}', String(workflow.warnings.length))}
          </p>
        ) : null}
      </div>

      {/* Right: actions */}
      <div className="flex flex-shrink-0 items-center gap-1">
        <button
          type="button"
          onClick={() => onView(workflow)}
          data-testid={testId ? `${testId}-view` : undefined}
          className="rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-1.5 text-xs font-medium text-stone-700 dark:text-neutral-200 transition-colors hover:bg-stone-50 dark:hover:bg-neutral-800 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-500">
          {t('common.seeAll')}
        </button>
        {canDelete && onDelete ? (
          <button
            type="button"
            onClick={() => onDelete(workflow)}
            data-testid={testId ? `${testId}-delete` : undefined}
            aria-label={t('workflows.delete')}
            className="flex h-7 w-7 items-center justify-center rounded-lg text-stone-400 dark:text-neutral-500 transition-colors hover:bg-coral-50 dark:hover:bg-coral-500/10 hover:text-coral-600 dark:hover:text-coral-400 focus:outline-none focus-visible:ring-2 focus-visible:ring-coral-500">
            <svg
              className="h-3.5 w-3.5"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M3 6h18M8 6V4a2 2 0 012-2h4a2 2 0 012 2v2m3 0v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6h14z"
              />
            </svg>
          </button>
        ) : null}
      </div>
    </div>
  );
};

export default WorkflowCard;
