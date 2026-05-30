/**
 * PhaseEditor
 * -----------
 * Read-only display of a single workflow phase. Shows rules, scripts,
 * tool allow/deny lists, and context providers. Editing is not supported
 * in v1 — workflows are defined by the core on disk.
 */
import { type FC } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { ToolScope, WorkflowPhase } from '../../services/api/workflowsApi';

interface PhaseEditorProps {
  phaseName: string;
  phase: WorkflowPhase;
}

function StringList({ items, emptyKey }: { items: string[]; emptyKey: string }) {
  const { t } = useT();
  if (items.length === 0) {
    return <p className="text-xs italic text-stone-400 dark:text-neutral-500">{t(emptyKey)}</p>;
  }
  return (
    <ul className="mt-1 space-y-1">
      {items.map((item, i) => (
        <li
          key={i}
          className="rounded-md bg-stone-50 dark:bg-neutral-800/60 border border-stone-200 dark:border-neutral-700 px-2 py-1 font-mono text-[11px] text-stone-700 dark:text-neutral-200">
          {item}
        </li>
      ))}
    </ul>
  );
}

function ToolScopeDisplay({ scope }: { scope: ToolScope | null | undefined }) {
  const { t } = useT();
  if (!scope) {
    return (
      <p className="text-xs italic text-stone-400 dark:text-neutral-500">
        {t('workflows.phase.toolScope.inherited')}
      </p>
    );
  }
  return (
    <div className="space-y-2">
      {scope.allow.length > 0 ? (
        <div>
          <p className="text-[10px] font-semibold uppercase tracking-wide text-sage-600 dark:text-sage-400">
            {t('workflows.phase.toolScope.allow')}
          </p>
          <div className="mt-1 flex flex-wrap gap-1">
            {scope.allow.map(tool => (
              <span
                key={tool}
                className="inline-flex items-center rounded border border-sage-200 dark:border-sage-500/30 bg-sage-50 dark:bg-sage-500/10 px-1.5 py-0.5 font-mono text-[10px] text-sage-700 dark:text-sage-300">
                {tool}
              </span>
            ))}
          </div>
        </div>
      ) : null}
      {scope.deny.length > 0 ? (
        <div>
          <p className="text-[10px] font-semibold uppercase tracking-wide text-coral-600 dark:text-coral-400">
            {t('workflows.phase.toolScope.deny')}
          </p>
          <div className="mt-1 flex flex-wrap gap-1">
            {scope.deny.map(tool => (
              <span
                key={tool}
                className="inline-flex items-center rounded border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-1.5 py-0.5 font-mono text-[10px] text-coral-700 dark:text-coral-300">
                {tool}
              </span>
            ))}
          </div>
        </div>
      ) : null}
      {scope.allow.length === 0 && scope.deny.length === 0 ? (
        <p className="text-xs italic text-stone-400 dark:text-neutral-500">
          {t('workflows.phase.toolScope.empty')}
        </p>
      ) : null}
    </div>
  );
}

const PhaseEditor: FC<PhaseEditorProps> = ({ phaseName, phase }) => {
  const { t } = useT();

  return (
    <div className="rounded-xl border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 p-4 space-y-4">
      {/* Phase name */}
      <div className="flex items-center gap-2">
        <span className="inline-flex items-center rounded-full border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-2.5 py-0.5 font-mono text-xs font-medium text-primary-700 dark:text-primary-300">
          {phaseName}
        </span>
        {phase.description ? (
          <p className="text-xs text-stone-500 dark:text-neutral-400">{phase.description}</p>
        ) : null}
      </div>

      {/* Rules */}
      <div>
        <h4 className="text-[10px] font-semibold uppercase tracking-wide text-stone-500 dark:text-neutral-400">
          {t('workflows.phase.rules')} ({phase.rules.length})
        </h4>
        <StringList items={phase.rules} emptyKey="workflows.phase.rules.empty" />
      </div>

      {/* Scripts */}
      <div>
        <h4 className="text-[10px] font-semibold uppercase tracking-wide text-stone-500 dark:text-neutral-400">
          {t('workflows.phase.scripts')} ({phase.scripts.length})
        </h4>
        <StringList items={phase.scripts} emptyKey="workflows.phase.scripts.empty" />
      </div>

      {/* Tool scope */}
      <div>
        <h4 className="text-[10px] font-semibold uppercase tracking-wide text-stone-500 dark:text-neutral-400">
          {t('workflows.phase.toolScope')}
        </h4>
        <div className="mt-1">
          <ToolScopeDisplay scope={phase.tools} />
        </div>
      </div>

      {/* Context */}
      <div>
        <h4 className="text-[10px] font-semibold uppercase tracking-wide text-stone-500 dark:text-neutral-400">
          {t('workflows.phase.context')} ({phase.context.length})
        </h4>
        <StringList items={phase.context} emptyKey="workflows.phase.context.empty" />
      </div>
    </div>
  );
};

export default PhaseEditor;
