import WorktreeActions from '../../../components/worktree/WorktreeActions';
import { useT } from '../../../lib/i18n/I18nContext';
import type { SubagentActivity, ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import { basename } from '../../../utils/pathUtils';
import { formatTimelineEntry, formatToolName } from '../../../utils/toolTimelineFormatting';
import { parseWorkerThreadRef } from '../utils/workerThreadRef';
import { agentNameTone, AgentTimelineRail } from './AgentTimelineRail';
import { WorkerThreadRefCard, type WorkerThreadStatus } from './WorkerThreadRefCard';

/**
 * Map a parent timeline entry's status to the worker-thread lifecycle
 * phase rendered on `WorkerThreadRefCard`. The parent entry is what the
 * subagent_spawned / subagent_completed / subagent_failed socket events
 * mutate, so reading from it keeps the badge and the surrounding
 * `<details>` status pill in lockstep without a second source of truth.
 *
 * Returns `undefined` for the rare ambiguous case so the card stays
 * label-only rather than render a misleading state.
 */
function workerStatusFromEntry(
  status: ToolTimelineEntry['status']
): WorkerThreadStatus | undefined {
  if (status === 'running') return 'running';
  if (status === 'success') return 'completed';
  if (status === 'error') return 'failed';
  return undefined;
}

/**
 * Render the live activity of one running (or completed) sub-agent
 * inside its parent timeline row — the mode/dedicated-thread badge,
 * the child iteration counter, the final-run statistics, and the
 * flat list of child tool calls the sub-agent has executed.
 *
 * Kept as a sibling of the existing worker-thread / detail block so
 * the surrounding `<details>` chevron + status pill behaviour is
 * unaffected — this component only renders when `subagent` is
 * present on the entry, which is true for any row produced by the
 * `subagent_*` socket events from a current core.
 */
/** Chars of streamed subagent text/thinking shown in the inline card tail. */
const SUBAGENT_PREVIEW_CHARS = 140;

export function SubagentActivityBlock({
  subagent,
  onView,
}: {
  subagent: SubagentActivity;
  /** Opens the full-transcript drawer for this subagent. Omitted in
   * read-only contexts (e.g. a completed snapshot with no live driver). */
  onView?: () => void;
}) {
  const { t } = useT();
  const headerBits: string[] = [];
  if (subagent.mode) headerBits.push(subagent.mode);
  if (subagent.dedicatedThread) headerBits.push(t('conversations.toolTimeline.workerThread'));
  if (subagent.childIteration != null) {
    if (subagent.childMaxIterations != null) {
      headerBits.push(
        `${t('conversations.toolTimeline.turn')} ${subagent.childIteration}/${subagent.childMaxIterations}`
      );
    } else {
      headerBits.push(`${t('conversations.toolTimeline.step')} ${subagent.childIteration}`);
    }
  } else if (subagent.iterations != null) {
    headerBits.push(
      subagent.iterations === 1
        ? `${subagent.iterations} ${t('chat.turn')}`
        : `${subagent.iterations} ${t('chat.turns')}`
    );
  }
  if (subagent.elapsedMs != null) {
    headerBits.push(
      subagent.elapsedMs >= 1000
        ? `${(subagent.elapsedMs / 1000).toFixed(1)}s`
        : `${subagent.elapsedMs}ms`
    );
  }

  // Live one-line preview of the subagent's streamed processing, derived
  // from the ordered transcript: prefer the latest visible-output tail, then
  // fall back to the latest reasoning tail while the child is still thinking
  // and hasn't emitted visible text yet. Drives the at-a-glance "what is it
  // doing right now" affordance on the card.
  const transcript = subagent.transcript ?? [];
  const lastTextItem = [...transcript].reverse().find(i => i.kind === 'text');
  const lastThinkingItem = [...transcript].reverse().find(i => i.kind === 'thinking');
  const previewItem = lastTextItem ?? lastThinkingItem;
  const previewIcon = previewItem?.kind === 'text' ? '📝' : '💭';
  const preview =
    previewItem && 'text' in previewItem
      ? previewItem.text.replace(/\s+/g, ' ').trim().slice(-SUBAGENT_PREVIEW_CHARS)
      : '';

  return (
    <div
      className="mt-1 space-y-0.5 text-[10px] text-stone-500 dark:text-neutral-400"
      data-testid="subagent-activity">
      {headerBits.length > 0 ? (
        <div className="flex flex-wrap items-center gap-1.5">
          {headerBits.map(bit => (
            <span
              key={bit}
              className="rounded-full bg-stone-100 dark:bg-neutral-800 px-1.5 py-0.5 font-medium text-stone-600 dark:text-neutral-300">
              {bit}
            </span>
          ))}
        </div>
      ) : null}
      {subagent.toolCalls.length > 0 ? (
        <ul className="ml-1 space-y-0.5">
          {subagent.toolCalls.map(call => {
            const tone =
              call.status === 'running'
                ? 'text-amber-700 dark:text-amber-300'
                : call.status === 'success'
                  ? 'text-sage-700 dark:text-sage-300'
                  : 'text-coral-700 dark:text-coral-300';
            return (
              <li
                key={call.callId}
                className="flex items-center gap-1.5"
                data-testid="subagent-tool-call">
                <span className={`text-[9px] ${tone}`}>•</span>
                <span className="text-[10px] text-stone-700 dark:text-neutral-200">
                  {formatToolName(call.toolName)}
                </span>
                {call.iteration != null ? (
                  <span className="text-[9px] text-stone-400 dark:text-neutral-500">
                    ·t{call.iteration}
                  </span>
                ) : null}
                <span className={`text-[9px] ${tone}`}>{call.status}</span>
                {call.elapsedMs != null && call.status !== 'running' ? (
                  <span className="text-[9px] text-stone-400 dark:text-neutral-500">
                    {call.elapsedMs >= 1000
                      ? `${(call.elapsedMs / 1000).toFixed(1)}s`
                      : `${call.elapsedMs}ms`}
                  </span>
                ) : null}
              </li>
            );
          })}
        </ul>
      ) : null}
      {preview ? (
        <div
          className="flex items-start gap-1 text-[10px] text-stone-500 dark:text-neutral-400"
          data-testid="subagent-preview">
          <span aria-hidden>{previewIcon}</span>
          <span className="line-clamp-2 break-words italic">{preview}</span>
        </div>
      ) : null}
      {subagent.worktreePath ? (
        <div
          className="mt-1 space-y-1 rounded-md border border-stone-200 bg-stone-50/70 p-1.5 dark:border-neutral-800 dark:bg-neutral-900/50"
          data-testid="subagent-worktree">
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="font-medium text-stone-600 dark:text-neutral-300">
              {t('worktree.label')}
            </span>
            <span
              className="truncate font-mono text-[10px] text-stone-500 dark:text-neutral-400"
              title={subagent.worktreePath}>
              {basename(subagent.worktreePath)}
            </span>
            {subagent.isDirty ? (
              <span className="rounded-full bg-amber-100 px-1.5 py-0.5 text-[9px] font-medium text-amber-700 dark:bg-amber-500/15 dark:text-amber-300">
                {t('worktree.dirty')}
              </span>
            ) : (
              <span className="rounded-full bg-sage-100 px-1.5 py-0.5 text-[9px] font-medium text-sage-700 dark:bg-sage-500/15 dark:text-sage-300">
                {t('worktree.clean')}
              </span>
            )}
            {subagent.changedFiles && subagent.changedFiles.length > 0 ? (
              <span className="text-[9px] text-stone-400 dark:text-neutral-500">
                {subagent.changedFiles.length}{' '}
                {subagent.changedFiles.length === 1
                  ? t('worktree.changedFile')
                  : t('worktree.changedFiles')}
              </span>
            ) : null}
          </div>
          <WorktreeActions path={subagent.worktreePath} isDirty={subagent.isDirty} compact />
        </div>
      ) : null}
      {onView ? (
        <button
          type="button"
          onClick={onView}
          data-testid="subagent-view-processing"
          className="mt-0.5 rounded-full px-1.5 py-0.5 text-[10px] font-medium text-primary-600 hover:bg-primary-50 dark:text-primary-300 dark:hover:bg-primary-500/15">
          {t('conversations.subagent.viewProcessing')} →
        </button>
      ) : null}
    </div>
  );
}

function normalizeToolBody(value?: string): string | undefined {
  if (!value) return undefined;
  const trimmed = value.trim();
  if (trimmed.length === 0) return undefined;
  if (trimmed === '{}' || trimmed === '[]' || trimmed === 'null') return undefined;
  return value;
}

/**
 * Neutral surface tones for an expanded row's body (worker-thread card,
 * detail bubble, code block). Per the Figma "Agentic task insights"
 * design these read as plain light cards rather than status-coloured
 * panels — the row's *status* is conveyed by the agent name (see
 * {@link agentNameTone}), so the body stays visually quiet.
 */
const BODY_SURFACE = 'bg-stone-50 dark:bg-neutral-800/60';

/**
 * The agent-run timeline rendered above an assistant answer — the
 * "Agentic task insights" surface from the Figma Chat design.
 *
 * Each {@link ToolTimelineEntry} is a row on a shared vertical timeline
 * rail ({@link AgentTimelineRail}); the agent name carries the run state
 * (pulsing while in flight, solid when done) and expands in place to show
 * its detail/code/sub-agent activity. The whole group sits under a
 * collapsible "⚙️ Working… / Agentic task insights" header so the user can
 * fold the live activity away.
 */
export function ToolTimelineBlock({
  entries,
  onViewSubagent,
  expandAllRows = false,
}: {
  entries: ToolTimelineEntry[];
  /** Opens the full-transcript drawer for a subagent row. When omitted,
   * subagent cards render without the "view full processing" affordance
   * (e.g. interrupted-snapshot rendering with no live driver). */
  onViewSubagent?: (subagent: SubagentActivity) => void;
  /** Expand every row's details by default (used by the "Agent Process
   * Source" panel, where the whole run should be visible at a glance).
   * In the inline chat only the latest running row auto-expands. */
  expandAllRows?: boolean;
}) {
  const { t } = useT();
  const latestRunningEntryId = [...entries].reverse().find(entry => entry.status === 'running')?.id;

  if (entries.length === 0) return null;

  // The group header is a static section label — the live "working" state is
  // conveyed by the pulsing agent-name rows (and the chat's own activity
  // indicator), so the header does NOT repeat a "Working…" string.
  return (
    <details open className="group/insights mb-2 px-1 py-0" data-testid="agent-task-insights">
      <summary className="mb-1.5 flex cursor-pointer list-none items-center gap-1.5 select-none marker:hidden">
        <span className="text-[11px] font-medium text-stone-500 dark:text-neutral-400">
          {t('conversations.agentTaskInsights.title')}
        </span>
        <span className="text-[9px] text-stone-400 transition-transform group-open/insights:rotate-90 dark:text-neutral-500">
          ▶
        </span>
      </summary>
      <div className="text-xs text-stone-400 dark:text-neutral-500">
        {entries.map((entry, index) => {
          const formatted = formatTimelineEntry(entry);
          const detailContent =
            normalizeToolBody(formatted.detail) ?? normalizeToolBody(entry.argsBuffer);
          const workerRef = parseWorkerThreadRef(formatted.detail ?? entry.detail);
          const subagent = entry.subagent;
          // A subagent row should always render the expandable details so
          // its live activity is visible — even when there is no prompt
          // detail to show. Mirrors the rule that a non-subagent row only
          // expands when it has detail content.
          const expandable = detailContent != null || subagent != null;
          const shouldAutoExpand =
            expandAllRows || (latestRunningEntryId != null && latestRunningEntryId === entry.id);
          const nameTone = agentNameTone(entry.status);

          return (
            <AgentTimelineRail
              key={entry.id}
              isFirst={index === 0}
              isLast={index === entries.length - 1}>
              {expandable ? (
                <details open={shouldAutoExpand} className="group/row">
                  <summary className="flex cursor-pointer list-none items-center gap-1.5 select-none marker:hidden">
                    <span className={`text-[11px] font-medium ${nameTone}`}>{formatted.title}</span>
                    <span className="text-[9px] text-stone-300 transition-transform group-open/row:rotate-90 dark:text-neutral-600">
                      ▶
                    </span>
                  </summary>
                  {workerRef ? (
                    <div
                      className={`mt-1 rounded-xl rounded-tl-md px-2.5 py-2 text-[11px] whitespace-pre-wrap break-words text-stone-600 dark:text-neutral-300 ${BODY_SURFACE}`}>
                      {workerRef.before}
                      <WorkerThreadRefCard
                        ref={workerRef.ref}
                        status={workerStatusFromEntry(entry.status)}
                      />
                      {workerRef.after ? <div className="mt-1">{workerRef.after}</div> : null}
                    </div>
                  ) : formatted.detail ? (
                    <div
                      className={`mt-1 rounded-xl rounded-tl-md px-2.5 py-2 text-[11px] whitespace-pre-wrap break-words text-stone-600 dark:text-neutral-300 ${BODY_SURFACE}`}>
                      {formatted.detail}
                    </div>
                  ) : detailContent ? (
                    <pre
                      className={`mt-1 max-h-24 overflow-y-auto rounded px-2 py-1 font-mono text-[10px] whitespace-pre-wrap break-all text-stone-600 dark:text-neutral-300 ${BODY_SURFACE}`}>
                      {detailContent}
                    </pre>
                  ) : null}
                  {subagent ? (
                    <SubagentActivityBlock
                      subagent={subagent}
                      onView={onViewSubagent ? () => onViewSubagent(subagent) : undefined}
                    />
                  ) : null}
                </details>
              ) : (
                <div className="flex items-center">
                  <span className={`text-[11px] font-medium ${nameTone}`}>{formatted.title}</span>
                </div>
              )}
            </AgentTimelineRail>
          );
        })}
      </div>
    </details>
  );
}
