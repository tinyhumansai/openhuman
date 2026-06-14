import { useEffect } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import type { ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import { type AgentSource, extractAgentSources } from '../../../utils/toolTimelineFormatting';
import { AgentSparkIcon } from './AgentTimelineRail';
import { ToolTimelineBlock } from './ToolTimelineBlock';

/** Compact globe glyph for a source row. Inherits `currentColor`. */
function GlobeIcon({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 12 12"
      width="12"
      height="12"
      aria-hidden
      className={className}
      focusable="false">
      <circle cx="6" cy="6" r="5" fill="none" stroke="currentColor" strokeWidth="1" />
      <path
        d="M1 6h10M6 1c1.8 1.4 1.8 8.6 0 10M6 1c-1.8 1.4-1.8 8.6 0 10"
        fill="none"
        stroke="currentColor"
        strokeWidth="1"
      />
    </svg>
  );
}

/** One web-source row: globe + hostname title (left) + full URL (right). */
function AgentSourceRow({ source }: { source: AgentSource }) {
  return (
    <li>
      <a
        href={source.url}
        target="_blank"
        rel="noreferrer noopener"
        className="flex items-center justify-between gap-3 rounded-md px-1.5 py-1 text-[11px] hover:bg-stone-50 dark:hover:bg-neutral-800/60"
        data-testid="agent-source-row">
        <span className="flex min-w-0 items-center gap-1.5">
          <GlobeIcon className="shrink-0 text-stone-400 dark:text-neutral-500" />
          <span className="truncate text-stone-700 dark:text-neutral-200">{source.title}</span>
        </span>
        <span className="shrink-0 truncate text-stone-400 dark:text-neutral-500">{source.url}</span>
      </a>
    </li>
  );
}

/**
 * The consolidated "Agent Process Source" side panel from the Figma Chat
 * design — slid in from the right (~600px) when the user clicks
 * "View full agent process Source →" beneath a settled answer.
 *
 * Unlike {@link SubagentDrawer} (which drills into one sub-agent's live
 * transcript), this panel shows the *whole* run: the full agent-insights
 * timeline plus the distinct web sources the agents visited. It reuses
 * {@link ToolTimelineBlock} as a single source of truth.
 *
 * Note: this panel IS the full-processing view, so it does NOT forward an
 * `onViewSubagent` handler — the rows render without the redundant
 * "view full processing →" affordance.
 */
export function AgentProcessSourcePanel({
  open,
  entries,
  onClose,
}: {
  open: boolean;
  entries: ToolTimelineEntry[];
  onClose: () => void;
}) {
  const { t } = useT();

  // Close on Escape for keyboard parity with the backdrop click.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  if (!open) return null;

  const sources = extractAgentSources(entries);

  return (
    <div className="fixed inset-0 z-50 flex justify-end" data-testid="agent-process-source-panel">
      {/* Backdrop */}
      <button
        type="button"
        aria-label={t('conversations.subagent.close')}
        className="absolute inset-0 bg-stone-900/30 dark:bg-black/50"
        onClick={onClose}
      />
      <aside className="relative flex h-full w-full max-w-[600px] flex-col bg-white shadow-xl dark:bg-neutral-900">
        {/* Header */}
        <header className="flex items-center gap-2.5 border-b border-stone-200 px-4 py-3 dark:border-neutral-800">
          <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary-50 text-primary-500 dark:bg-primary-500/15">
            <AgentSparkIcon />
          </span>
          <span className="min-w-0 flex-1 truncate font-semibold text-stone-800 dark:text-neutral-100">
            {t('conversations.agentTaskInsights.processSourceTitle')}
          </span>
          <button
            type="button"
            onClick={onClose}
            aria-label={t('conversations.subagent.close')}
            className="shrink-0 rounded-full p-1.5 text-stone-400 hover:bg-stone-100 hover:text-stone-600 dark:hover:bg-neutral-800 dark:hover:text-neutral-200">
            ✕
          </button>
        </header>

        {/* Body — the full agent timeline, then the visited sources. */}
        <div className="flex-1 space-y-5 overflow-y-auto px-4 py-4">
          <section>
            <h3 className="mb-2 text-[10px] font-semibold tracking-wide text-stone-400 uppercase dark:text-neutral-500">
              {t('conversations.agentTaskInsights.stepsHeading')}
            </h3>
            {entries.length > 0 ? (
              <ToolTimelineBlock entries={entries} expandAllRows />
            ) : (
              <p className="text-xs text-stone-400 italic dark:text-neutral-500">
                {t('conversations.agentTaskInsights.noSteps')}
              </p>
            )}
          </section>

          {sources.length > 0 ? (
            <section>
              <h3 className="mb-2 text-[10px] font-semibold tracking-wide text-stone-400 uppercase dark:text-neutral-500">
                {t('conversations.agentTaskInsights.sourcesHeading')}
              </h3>
              <ul className="space-y-0.5">
                {sources.map(source => (
                  <AgentSourceRow key={source.id} source={source} />
                ))}
              </ul>
            </section>
          ) : null}
        </div>
      </aside>
    </div>
  );
}
