import type { ProcessingTranscriptItem, ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import { formatTimelineEntry } from '../../../utils/toolTimelineFormatting';
import { ProcessingTranscriptView } from './ProcessingTranscriptView';
import { SubagentActivityBlock, ToolTimelineBlock } from './ToolTimelineBlock';

/**
 * The collapsed process trail rendered above a PAST (settled) turn's answer on a
 * reopened thread — restore-fidelity fix 1.
 *
 * A restored turn must show the *same* interleaved thoughts + tools view a live
 * turn does, not just its tool cards: the persisted `transcript`
 * (narration/thinking/tool pointers, hydrated into
 * `turnTranscriptsByThread`) drives {@link ProcessingTranscriptView}, so the
 * reasoning and narration replay inline exactly where they streamed. Restored
 * sub-agent transcripts (fix 4) render beneath as their own activity blocks,
 * mirroring the whole-run {@link AgentProcessSourcePanel} body.
 *
 * Legacy turns persisted before the transcript field existed have no
 * `transcript` — they fall back to the tool-only {@link ToolTimelineBlock}, so
 * older threads keep rendering unchanged.
 */
export function PastTurnInsights({
  entries,
  transcript,
}: {
  entries: ToolTimelineEntry[];
  transcript: ProcessingTranscriptItem[];
}) {
  // No reasoning/narration trail persisted (legacy snapshot): render the
  // tool-only timeline, which already nests each sub-agent's activity inline.
  if (transcript.length === 0) {
    return <ToolTimelineBlock entries={entries} />;
  }

  const subagentEntries = entries.filter(entry => entry.subagent);

  return (
    <div className="space-y-3">
      {/* Interleaved narration + hidden reasoning + grouped tool steps. */}
      <ProcessingTranscriptView transcript={transcript} entries={entries} />

      {/* Sub-agents — each delegated agent's restored transcript (thoughts +
          tool rows), so a reopened turn keeps its sub-agent reasoning, not just
          a flat tool row. `ProcessingTranscriptView` shows the spawn as a tool
          row but does not nest the child's activity, so surface it here. */}
      {subagentEntries.length > 0 ? (
        <div className="space-y-3" data-testid="past-turn-subagents">
          {subagentEntries.map(entry => (
            <div key={entry.id}>
              <p className="text-[12px] font-medium text-content-secondary">
                {formatTimelineEntry(entry).title}
              </p>
              <SubagentActivityBlock subagent={entry.subagent!} />
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}
