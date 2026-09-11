import { useT } from '../../../../lib/i18n/I18nContext';
import type {
  ProcessingTranscriptItem,
  ToolTimelineEntry,
} from '../../../../store/chatRuntimeSlice';
import { ToolTimelineBlock } from '../ToolTimelineBlock';

export interface AgentInsightsSlotProps {
  entries: ToolTimelineEntry[];
  transcript: ProcessingTranscriptItem[];
  /** True while a turn is in flight on this thread. */
  turnActive: boolean;
  /** True when the settled timeline renders above the latest agent answer. */
  timelineBeforeLatestAnswer: boolean;
  /** `themeSlice.hideAgentInsights` — suppresses the verbose step rows. */
  hideAgentInsights: boolean;
  onViewDetails: (entry: ToolTimelineEntry) => void;
  onViewWholeRun: () => void;
}

/**
 * The "Agentic task insights" slot: the inline step timeline plus the openers
 * that reach the full Agent Process Source panel.
 *
 * Extracted from `ChatThreadView` verbatim — every `data-testid` here
 * (`agent-processing-link`, `agent-process-source-fallback`,
 * `view-process-source`) is selected on by WDIO specs and must not move.
 *
 * Returns `null` when the thread has neither tool steps nor a persisted
 * reasoning/narration transcript. The transcript guard is not redundant: a
 * tool-less turn (the agent only thinks or narrates) has an empty timeline but
 * still persists thoughts, and without it those thoughts are unreachable.
 */
export function AgentInsightsSlot({
  entries,
  transcript,
  turnActive,
  timelineBeforeLatestAnswer,
  hideAgentInsights,
  onViewDetails,
  onViewWholeRun,
}: AgentInsightsSlotProps) {
  const { t } = useT();
  if (entries.length === 0 && transcript.length === 0) return null;

  return (
    <>
      {hideAgentInsights ? (
        // "Hide agent thinking" is ON: suppress the verbose step rows. While in
        // flight, surface a compact blinking "Processing" link; once settled the
        // "View full agent process Source" opener below takes over (so only
        // render this fallback when that opener won't).
        turnActive ? (
          <button
            type="button"
            onClick={onViewWholeRun}
            data-testid="agent-processing-link"
            className="flex items-center gap-1.5 px-1 py-1 text-[11px] font-medium text-primary-600 hover:underline dark:text-primary-300">
            <span className="inline-block w-1.5 h-1.5 rounded-full bg-primary-400 animate-pulse" />
            <span>{t('conversations.agentTaskInsights.processing')} →</span>
          </button>
        ) : !timelineBeforeLatestAnswer ? (
          <button
            type="button"
            onClick={onViewWholeRun}
            data-testid="agent-process-source-fallback"
            className="px-1 text-[11px] font-medium text-primary-600 hover:underline dark:text-primary-300">
            {t('conversations.agentTaskInsights.viewProcessSource')} →
          </button>
        ) : null
      ) : entries.length > 0 ? (
        <ToolTimelineBlock
          entries={entries}
          onViewDetails={onViewDetails}
          onViewWholeRun={onViewWholeRun}
          // `turnActive` is the caller's `isSending`, deliberately NOT a raw
          // `in` membership check on `inferenceTurnLifecycleByThread`: that map
          // also carries `'interrupted'` entries (a turn that crashed mid-flight
          // in a PRIOR core process, written by `hydrateRuntimeFromSnapshot` on
          // cold boot) which have no live driver and must NOT read as an active
          // turn, or stale disclosure state leaks into a later retry.
          turnActive={turnActive}
          // Interleaved narration + thinking + tool steps in stream order.
          // Renders through the same `ProcessingTranscriptView` the Agent
          // Process Source panel uses, so the rail IS a windowed view of the
          // panel rather than a second, divergent rendering of the turn.
          transcript={transcript}
        />
      ) : (
        // Transcript-only turn: reasoning/narration was streamed but no tool
        // calls were made, so the inline step timeline is empty. The thoughts
        // are still persisted — surface a standalone opener (matching the
        // settled insights header) so the full-run panel stays reachable.
        <button
          type="button"
          onClick={onViewWholeRun}
          data-testid="view-process-source"
          className="flex items-center gap-1.5 px-1 py-1 text-left">
          <span className="text-[13px] font-medium text-content-muted">
            {t('conversations.agentTaskInsights.title')}
          </span>
          <span className="text-[13px] font-medium text-primary-600 dark:text-primary-300">→</span>
        </button>
      )}
      {/* "View full agent process Source" — only needed in the hidden-insights
          settled state; when the timeline is visible the link lives in its
          header (ToolTimelineBlock onViewWholeRun). */}
      {timelineBeforeLatestAnswer && hideAgentInsights && (
        <button
          type="button"
          onClick={onViewWholeRun}
          data-testid="view-process-source"
          className="px-1 text-[11px] font-medium text-primary-600 hover:underline dark:text-primary-300">
          {t('conversations.agentTaskInsights.viewProcessSource')} →
        </button>
      )}
    </>
  );
}
