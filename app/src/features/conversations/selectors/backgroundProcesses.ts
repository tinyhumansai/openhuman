import type {
  SubagentActivity,
  ToolTimelineEntry,
  ToolTimelineEntryStatus,
} from '../../../store/chatRuntimeSlice';

/**
 * A background process = a *detached* sub-agent spawned with
 * `spawn_subagent` (a fire-and-forget tokio task that keeps running after
 * the parent turn returns). The backend marks these with `mode: "async"` on the
 * `SubagentSpawned` event (every blocking spawn emits `mode: "typed"`), and the
 * frontend carries it through on {@link SubagentActivity.mode}. So the whole
 * "is this truly in the background?" question reduces to `mode === 'async'`.
 *
 * Moved out of the (now-deleted) `BackgroundProcessesPanel.tsx` when that
 * panel was replaced by the vendored `BackgroundInbox` element
 * (`aui/BackgroundInboxCard.tsx`) — this selector and its `BackgroundProcess`
 * type are unrelated to any one host component.
 */
export interface BackgroundProcess {
  taskId: string;
  name: string;
  goal: string;
  status: ToolTimelineEntryStatus;
  toolCount: number;
  iterations?: number;
  /** Live/settled elapsed time, when the core reports one, for the inbox's elapsed column. */
  elapsedMs?: number;
}

const subagentName = (s: SubagentActivity): string =>
  (s.displayName && s.displayName.trim()) || s.agentId || 'sub-agent';

/**
 * Pure selector: the detached background sub-agents spawned in a thread,
 * newest-relevant first, deduped by spawn `taskId`. Driven off the same tool
 * timeline the inline rows use, so a process opened here resolves to the
 * exact same entry in the Agent Process Source panel.
 */
export function selectBackgroundProcesses(timeline: ToolTimelineEntry[]): BackgroundProcess[] {
  const seen = new Set<string>();
  const out: BackgroundProcess[] = [];
  for (const entry of timeline) {
    const sub = entry.subagent;
    if (!sub || sub.mode !== 'async') continue;
    if (seen.has(sub.taskId)) continue;
    seen.add(sub.taskId);
    out.push({
      taskId: sub.taskId,
      name: subagentName(sub),
      goal: (sub.prompt ?? '').trim(),
      status: entry.status,
      toolCount: sub.toolCalls?.length ?? 0,
      iterations: sub.iterations,
      elapsedMs: sub.elapsedMs,
    });
  }
  // Running first, so live work stays at the top of the list.
  return out.sort((a, b) => Number(b.status === 'running') - Number(a.status === 'running'));
}
