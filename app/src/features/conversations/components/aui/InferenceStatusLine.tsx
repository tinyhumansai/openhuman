import { useT } from '../../../../lib/i18n/I18nContext';
import type { ToolTimelineEntry } from '../../../../store/chatRuntimeSlice';
import { formatTimelineEntry } from '../../../../utils/toolTimelineFormatting';

export interface InferenceStatusLineProps {
  status: {
    phase: 'thinking' | 'tool_use' | 'subagent' | string;
    iteration: number;
    activeTool?: string;
    activeSubagent?: string;
  };
  /** The newest running non-subagent row, used to title the `tool_use` phase. */
  activeToolEntry?: ToolTimelineEntry;
  /** The running subagent row, used to title the `subagent` phase. */
  activeSubagentEntry?: ToolTimelineEntry;
}

/**
 * The one-line "what is the model doing right now" indicator.
 *
 * For the `tool_use` / `subagent` phases this only restates the active row the
 * agentic-task-insights timeline already shows, so the caller suppresses it
 * once that timeline is on screen and keeps it for the `thinking` phase (which
 * has no timeline row yet) or when there is no timeline to fall back on.
 */
export function InferenceStatusLine({
  status,
  activeToolEntry,
  activeSubagentEntry,
}: InferenceStatusLineProps) {
  const { t } = useT();
  return (
    <div
      data-testid="inference-status-line"
      className="flex items-center gap-2 px-1 py-1.5 text-xs text-content-muted">
      <span className="inline-block w-2 h-2 rounded-full bg-primary-400 animate-pulse" />
      <span>
        {status.phase === 'thinking' &&
          (status.iteration > 0
            ? t('chat.thinkingIteration').replace('{n}', String(status.iteration))
            : t('chat.thinkingDots'))}
        {status.phase === 'tool_use' &&
          `${
            formatTimelineEntry(
              activeToolEntry ?? {
                id: 'active-tool',
                name: status.activeTool ?? 'tool',
                round: status.iteration,
                seq: 0,
                status: 'running',
              }
            ).title
          }...`}
        {status.phase === 'subagent' &&
          `${
            formatTimelineEntry(
              activeSubagentEntry ?? {
                id: 'active-subagent',
                name: `subagent:${status.activeSubagent ?? ''}`,
                round: status.iteration,
                seq: 0,
                status: 'running',
              }
            ).title
          }...`}
      </span>
    </div>
  );
}
