import WorktreeActions from '../../../components/worktree/WorktreeActions';
import { useT } from '../../../lib/i18n/I18nContext';
import type {
  SubagentActivity,
  ToolTimelineEntry,
  ToolTimelineEntryStatus,
} from '../../../store/chatRuntimeSlice';
import { basename } from '../../../utils/pathUtils';
import {
  formatTimelineEntry,
  formatToolName,
  stripToolCallEnvelopes,
} from '../../../utils/toolTimelineFormatting';
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

/** Tone classes for a child tool-call row keyed by its lifecycle status. */
function toolCallTone(status: ToolTimelineEntryStatus): string {
  if (status === 'running') return 'text-amber-700 dark:text-amber-300';
  if (status === 'success') return 'text-sage-700 dark:text-sage-300';
  return 'text-coral-700 dark:text-coral-300';
}

/**
 * One child tool-call row in a sub-agent's inline activity. Shared by the
 * ordered transcript (interleaved with {@link ThoughtBlock}) and the flat
 * `toolCalls` fallback, so the row markup lives in exactly one place.
 */
function ToolCallRow({
  call,
}: {
  call: {
    callId: string;
    toolName: string;
    status: ToolTimelineEntryStatus;
    elapsedMs?: number;
    iteration?: number;
  };
}) {
  const tone = toolCallTone(call.status);
  return (
    <div className="flex items-center gap-1.5" data-testid="subagent-tool-call">
      <span className={`text-[9px] ${tone}`}>•</span>
      <span className="text-[10px] text-stone-700 dark:text-neutral-200">
        {formatToolName(call.toolName)}
      </span>
      {call.iteration != null ? (
        <span className="text-[9px] text-stone-400 dark:text-neutral-500">·t{call.iteration}</span>
      ) : null}
      <span className={`text-[9px] ${tone}`}>{call.status}</span>
      {call.elapsedMs != null && call.status !== 'running' ? (
        <span className="text-[9px] text-stone-400 dark:text-neutral-500">
          {call.elapsedMs >= 1000
            ? `${(call.elapsedMs / 1000).toFixed(1)}s`
            : `${call.elapsedMs}ms`}
        </span>
      ) : null}
    </div>
  );
}

/**
 * The agent's reasoning or visible narration, surfaced inline in the timeline
 * as a quoted/italic "Thoughts" block at the position it streamed — so a
 * thought shows up wherever it occurred between tool calls. Both `thinking`
 * and `text` transcript items render through here. Renders nothing for an
 * all-whitespace delta so a half-streamed item never flashes an empty quote.
 */
function ThoughtBlock({ text }: { text: string }) {
  const { t } = useT();
  // Drop any inline `<tool_call>…</tool_call>` envelope the model emitted as
  // text — the call already shows as its own row — then flatten to one line.
  const clean = stripToolCallEnvelopes(text).replace(/\s+/g, ' ').trim();
  if (!clean) return null;
  return (
    <details
      open
      data-testid="subagent-thought"
      className="group/thought my-0.5 border-l-2 border-stone-200 pl-2 dark:border-neutral-700">
      <summary className="flex cursor-pointer list-none items-center gap-1 select-none marker:hidden">
        <span aria-hidden className="text-[9px] leading-none">
          💭
        </span>
        <span className="text-[9px] font-semibold tracking-wide text-stone-400 uppercase dark:text-neutral-500">
          {t('conversations.subagent.thoughts')}
        </span>
        <span className="text-[8px] text-stone-300 transition-transform group-open/thought:rotate-90 dark:text-neutral-600">
          ▶
        </span>
      </summary>
      <div className="mt-0.5 text-[10px] break-words text-stone-500 italic dark:text-neutral-400">
        “{clean}”
      </div>
    </details>
  );
}

/** Tail of the parent's in-flight response shown in the processing panel. */
const RESPONSE_PREVIEW_CHARS = 320;

/**
 * The parent agent's live response, surfaced inside the processing panel while
 * the turn is in flight — its lead-in narration ("Let me check your Notion…")
 * belongs with the work it's narrating, not in a standalone chat bubble. The
 * final answer still lands in the message bubble once the turn settles.
 * Collapsible + accented apart from the stone-toned sub-agent Thoughts so the
 * parent's own voice reads as the primary thread.
 */
function LiveResponseBlock({ text }: { text: string }) {
  const { t } = useT();
  const clean = stripToolCallEnvelopes(text)
    .replace(/[ \t]+\n/g, '\n')
    .trimEnd();
  const shown = clean.slice(-RESPONSE_PREVIEW_CHARS);
  if (!shown.trim()) return null;
  return (
    <details
      open
      data-testid="agent-live-response"
      className="group/resp mt-1.5 border-l-2 border-primary-300 pl-2 dark:border-primary-500/50">
      <summary className="flex cursor-pointer list-none items-center gap-1 select-none marker:hidden">
        <span aria-hidden className="text-[9px] leading-none">
          💬
        </span>
        <span className="text-[9px] font-semibold tracking-wide text-primary-500 uppercase dark:text-primary-300">
          {t('conversations.agentTaskInsights.response')}
        </span>
        <span className="text-[8px] text-stone-300 transition-transform group-open/resp:rotate-90 dark:text-neutral-600">
          ▶
        </span>
      </summary>
      <p className="mt-0.5 text-[10px] leading-snug break-words whitespace-pre-wrap text-stone-600 dark:text-neutral-300">
        {clean.length > RESPONSE_PREVIEW_CHARS ? (
          <span className="text-stone-400 dark:text-neutral-500">…</span>
        ) : null}
        {shown}
        <span className="ml-0.5 inline-block h-3 w-1 animate-pulse bg-primary-400 align-middle" />
      </p>
    </details>
  );
}

/**
 * Render the live activity of one running (or completed) sub-agent inside its
 * parent timeline row — the mode/dedicated-thread badge, the child iteration
 * counter, the final-run statistics, and the ordered transcript of child tool
 * calls interleaved with the agent's "Thoughts" (reasoning + narration).
 *
 * Kept as a sibling of the existing worker-thread / detail block so the
 * surrounding `<details>` chevron + status pill behaviour is unaffected — this
 * component only renders when `subagent` is present on the entry, which is true
 * for any row produced by the `subagent_*` socket events from a current core.
 */
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

  // The ordered transcript drives the inline activity: child tool-call rows
  // and the agent's "Thoughts" (reasoning + visible narration) render in the
  // exact order they streamed, so each thought appears wherever it occurred
  // between tool calls. Falls back to the flat tool-call list when the prose
  // transcript is absent (e.g. a rehydrated/interrupted snapshot).
  const transcript = subagent.transcript ?? [];

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
      {transcript.length > 0 ? (
        <div className="ml-1 space-y-0.5" data-testid="subagent-transcript">
          {transcript.map((item, i) =>
            item.kind === 'tool' ? (
              <ToolCallRow key={item.callId} call={item} />
            ) : (
              <ThoughtBlock key={`thought-${i}`} text={item.text} />
            )
          )}
        </div>
      ) : subagent.toolCalls.length > 0 ? (
        <div className="ml-1 space-y-0.5">
          {subagent.toolCalls.map(call => (
            <ToolCallRow key={call.callId} call={call} />
          ))}
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
  liveResponse,
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
  /** The parent agent's in-flight response text. While the turn streams, its
   * narration renders inside this panel (as a "Response" block) instead of a
   * standalone chat bubble, so the lead-in sits with the work it narrates.
   * Omitted/empty once the turn settles — the final answer is the message
   * bubble. */
  liveResponse?: string;
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
      {liveResponse ? <LiveResponseBlock text={liveResponse} /> : null}
    </details>
  );
}
