import { useEffect } from 'react';

import Button from '../../../components/ui/Button';
import { useT } from '../../../lib/i18n/I18nContext';
import type { ProcessingTranscriptItem, ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import {
  type AgentSource,
  extractAgentSources,
  formatTimelineEntry,
} from '../../../utils/toolTimelineFormatting';
import { AgentSparkIcon } from './AgentTimelineRail';
import { ProcessingTranscriptView } from './ProcessingTranscriptView';
import { SubagentActivityBlock, ToolTimelineBlock } from './ToolTimelineBlock';

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
        className="flex items-center justify-between gap-3 rounded-md px-1.5 py-1 text-[11px] hover:bg-surface-hover"
        data-testid="agent-source-row">
        <span className="flex min-w-0 items-center gap-1.5">
          <GlobeIcon className="shrink-0 text-content-faint" />
          <span className="truncate text-content-secondary">{source.title}</span>
        </span>
        <span className="shrink-0 truncate text-content-faint">{source.url}</span>
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
  transcript = [],
  scopedEntry,
  onClose,
}: {
  open: boolean;
  entries: ToolTimelineEntry[];
  /** Ordered narration/thinking/tool transcript. When present, the panel
   *  renders the interleaved Hermes view; otherwise it falls back to the
   *  tool-only timeline. */
  transcript?: ProcessingTranscriptItem[];
  /** When set, the panel is scoped to a single step — its title becomes the
   *  step label and the body shows only that step's details (its sub-agent
   *  activity, or its tool detail). `undefined` → the whole-run overview. */
  scopedEntry?: ToolTimelineEntry;
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

  // Sources/sub-agents are scoped to the single step when one is selected,
  // else they cover the whole run.
  const sources = extractAgentSources(scopedEntry ? [scopedEntry] : entries);
  const subagentEntries = entries.filter(entry => entry.subagent);
  // For a scoped *non*-sub-agent step, the detail (args / output) to show.
  const scopedDetail = scopedEntry
    ? (formatTimelineEntry(scopedEntry).detail ?? scopedEntry.argsBuffer)
    : undefined;

  return (
    <div className="fixed inset-0 z-50 flex justify-end" data-testid="agent-process-source-panel">
      {/* Backdrop */}
      <button
        type="button"
        aria-label={t('conversations.subagent.close')}
        className="absolute inset-0 bg-stone-900/30 dark:bg-black/50"
        onClick={onClose}
      />
      <aside className="relative flex h-full w-full max-w-[600px] flex-col bg-surface shadow-xl">
        {/* Header */}
        <header className="flex items-center gap-2.5 border-b border-line px-4 py-3">
          <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary-50 text-primary-500 dark:bg-primary-500/15">
            <AgentSparkIcon />
          </span>
          <span className="min-w-0 flex-1 truncate font-semibold text-content">
            {scopedEntry
              ? formatTimelineEntry(scopedEntry).title
              : t('conversations.agentTaskInsights.processSourceTitle')}
          </span>
          <Button
            iconOnly
            variant="tertiary"
            size="sm"
            onClick={onClose}
            aria-label={t('conversations.subagent.close')}
            className="shrink-0 rounded-full">
            ✕
          </Button>
        </header>

        {/* Body — the full agent timeline, then the visited sources. */}
        <div className="flex-1 space-y-5 overflow-y-auto px-4 py-4">
          <section>
            <h3 className="mb-2 text-[10px] font-semibold tracking-wide text-content-faint uppercase">
              {t('conversations.agentTaskInsights.stepsHeading')}
            </h3>
            {scopedEntry ? (
              // Scoped to one step: show only that step's details.
              scopedEntry.subagent ? (
                <SubagentActivityBlock subagent={scopedEntry.subagent} />
              ) : scopedDetail ? (
                <pre className="max-h-[60vh] overflow-y-auto rounded-lg bg-surface-muted px-3 py-2 text-[12px] whitespace-pre-wrap break-words text-content-secondary">
                  {scopedDetail}
                </pre>
              ) : (
                <p className="text-xs text-content-faint italic">
                  {t('conversations.agentTaskInsights.noSteps')}
                </p>
              )
            ) : transcript.length > 0 ? (
              // Hermes-style interleaved narration + grouped, human-labeled steps.
              <ProcessingTranscriptView transcript={transcript} entries={entries} />
            ) : entries.length > 0 ? (
              // Legacy snapshot (no transcript): fall back to the tool timeline,
              // which already nests each sub-agent's full activity inline.
              <ToolTimelineBlock entries={entries} expandAllRows />
            ) : (
              <p className="text-xs text-content-faint italic">
                {t('conversations.agentTaskInsights.noSteps')}
              </p>
            )}
          </section>

          {/* Sub-agents — each delegated agent's full processing (thoughts +
              tool rows + detail). Only rendered alongside the transcript view,
              which doesn't nest sub-agent activity itself; the no-transcript
              fallback above already expands it. */}
          {!scopedEntry && transcript.length > 0 && subagentEntries.length > 0 ? (
            <section>
              <h3 className="mb-2 text-[10px] font-semibold tracking-wide text-content-faint uppercase">
                {t('conversations.agentTaskInsights.subagentsHeading')}
              </h3>
              <div className="space-y-3">
                {subagentEntries.map(entry => (
                  <div key={entry.id} data-testid="agent-source-subagent">
                    <p className="text-[12px] font-medium text-content-secondary">
                      {formatTimelineEntry(entry).title}
                    </p>
                    <SubagentActivityBlock subagent={entry.subagent!} />
                  </div>
                ))}
              </div>
            </section>
          ) : null}

          {sources.length > 0 ? (
            <section>
              <h3 className="mb-2 text-[10px] font-semibold tracking-wide text-content-faint uppercase">
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
