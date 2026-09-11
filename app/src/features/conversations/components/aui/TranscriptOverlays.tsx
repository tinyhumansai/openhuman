import { subagentApi } from '../../../../services/api/subagentApi';
import {
  markSubagentCancelled,
  type ProcessingTranscriptItem,
  type ToolTimelineEntry,
} from '../../../../store/chatRuntimeSlice';
import { useAppDispatch } from '../../../../store/hooks';
import { AgentProcessSourcePanel } from '../AgentProcessSourcePanel';
import { type BackgroundProcess, BackgroundProcessesPanel } from '../BackgroundProcessesPanel';
import { SubagentDrawer } from '../SubagentDrawer';

export interface TranscriptOverlaysProps {
  threadId: string | null;
  /** The thread's full live tool timeline — the Agent Process Source corpus. */
  entries: ToolTimelineEntry[];
  /** The thread's live reasoning/narration trail. */
  transcript: ProcessingTranscriptItem[];
  backgroundProcesses: BackgroundProcess[];
  showBackgroundProcesses: boolean;
  onCloseBackgroundProcesses: () => void;
  /** Spawn `taskId` of the sub-agent whose drawer is open, or `null`. */
  openSubagentTaskId: string | null;
  onOpenSubagent: (taskId: string | null) => void;
  showProcessSource: boolean;
  /** Scopes the process-source panel to one step; `undefined` = whole run. */
  scopedEntry?: ToolTimelineEntry;
  onCloseProcessSource: () => void;
}

/**
 * The three transcript-local modals: background sub-agents, the sub-agent
 * drawer, and the Agent Process Source panel.
 *
 * Split out of `ChatThreadView` because none of it is part of the transcript's
 * render path — it is driven entirely by the view's own disclosure state, and
 * keeping it inline made the hot path harder to read than it needs to be.
 */
export function TranscriptOverlays({
  threadId,
  entries,
  transcript,
  backgroundProcesses,
  showBackgroundProcesses,
  onCloseBackgroundProcesses,
  openSubagentTaskId,
  onOpenSubagent,
  showProcessSource,
  scopedEntry,
  onCloseProcessSource,
}: TranscriptOverlaysProps) {
  const dispatch = useAppDispatch();
  // Re-derived from the timeline on every render so the drawer streams
  // token-by-token as subagent_text_delta / subagent_thinking_delta events land
  // in Redux.
  const openSubagentEntry = openSubagentTaskId
    ? entries.find(entry => entry.subagent?.taskId === openSubagentTaskId)
    : undefined;

  return (
    <>
      <BackgroundProcessesPanel
        open={showBackgroundProcesses}
        processes={backgroundProcesses}
        onClose={onCloseBackgroundProcesses}
        onOpenProcess={taskId => {
          onCloseBackgroundProcesses();
          onOpenSubagent(taskId);
        }}
      />
      <SubagentDrawer
        key={openSubagentTaskId ?? 'none'}
        subagent={openSubagentEntry?.subagent ?? null}
        status={openSubagentEntry?.status}
        onCancel={
          openSubagentEntry?.subagent && threadId
            ? async () => {
                const taskId = openSubagentEntry.subagent!.taskId;
                const result = await subagentApi.cancel(taskId);
                // Only flip the row when something was actually aborted — a
                // cancelled=false result means the run already finished/unknown,
                // and overwriting its real terminal state would hide it. No
                // terminal socket event arrives for an aborted run, so the
                // optimistic mark is what surfaces the cancellation (the notice
                // itself reaches chat via the idle-gated delivery path).
                if (result.cancelled) {
                  dispatch(markSubagentCancelled({ threadId, taskId: result.taskId }));
                }
              }
            : undefined
        }
        onClose={() => onOpenSubagent(null)}
      />
      <AgentProcessSourcePanel
        open={showProcessSource}
        entries={entries}
        transcript={transcript}
        scopedEntry={scopedEntry}
        onClose={onCloseProcessSource}
      />
    </>
  );
}
