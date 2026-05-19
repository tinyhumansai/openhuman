import { useT } from '../../../lib/i18n/I18nContext';
import type { SubagentActivity, ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import { formatTimelineEntry } from '../../../utils/toolTimelineFormatting';
import { parseWorkerThreadRef } from '../utils/workerThreadRef';
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
export function SubagentActivityBlock({ subagent }: { subagent: SubagentActivity }) {
  const { t } = useT();
  const headerBits: string[] = [];
  if (subagent.mode) headerBits.push(subagent.mode);
  if (subagent.dedicatedThread) headerBits.push(t('conversations.toolTimeline.workerThread'));
  if (subagent.childIteration != null && subagent.childMaxIterations != null) {
    headerBits.push(
      `${t('conversations.toolTimeline.turn')} ${subagent.childIteration}/${subagent.childMaxIterations}`
    );
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
                <span className="font-mono text-[10px] text-stone-700 dark:text-neutral-200">
                  {call.toolName}
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
    </div>
  );
}

export function ToolTimelineBlock({ entries }: { entries: ToolTimelineEntry[] }) {
  const latestRunningEntryId = [...entries].reverse().find(entry => entry.status === 'running')?.id;

  const normalizeToolBody = (value?: string): string | undefined => {
    if (!value) return undefined;
    const trimmed = value.trim();
    if (trimmed.length === 0) return undefined;
    if (trimmed === '{}' || trimmed === '[]' || trimmed === 'null') return undefined;
    return value;
  };

  return (
    <div className="mb-2 space-y-1 px-1 py-0">
      {entries.map(entry => {
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
        const shouldAutoExpand = latestRunningEntryId != null && latestRunningEntryId === entry.id;
        const statusTone =
          entry.status === 'running'
            ? {
                pill: 'bg-amber-100 dark:bg-amber-500/20 text-amber-600 dark:text-amber-300',
                bubble: 'bg-amber-50 dark:bg-amber-500/10 text-amber-900 dark:text-amber-200',
                code: 'text-amber-800 dark:text-amber-300',
                chevron: 'text-amber-500 dark:text-amber-400',
              }
            : entry.status === 'success'
              ? {
                  pill: 'bg-sage-100 dark:bg-sage-500/20 text-sage-600 dark:text-sage-300',
                  bubble: 'bg-sage-50 dark:bg-sage-500/10 text-sage-900 dark:text-sage-200',
                  code: 'text-sage-800 dark:text-sage-300',
                  chevron: 'text-sage-500 dark:text-sage-400',
                }
              : {
                  pill: 'bg-coral-100 dark:bg-coral-500/20 text-coral-600 dark:text-coral-300',
                  bubble: 'bg-coral-50 dark:bg-coral-500/10 text-coral-900 dark:text-coral-200',
                  code: 'text-coral-800 dark:text-coral-300',
                  chevron: 'text-coral-500 dark:text-coral-400',
                };

        return (
          <div
            key={entry.id}
            className="flex flex-col gap-1 text-xs text-stone-400 dark:text-neutral-500">
            {expandable ? (
              <details open={shouldAutoExpand} className="ml-1 group">
                <summary className="flex cursor-pointer list-none items-center gap-2 select-none marker:hidden">
                  <span
                    className={`text-[10px] transition-transform group-open:rotate-90 ${statusTone.chevron}`}>
                    ▶
                  </span>
                  <span className="font-medium text-stone-600 dark:text-neutral-300">
                    {formatted.title}
                  </span>
                  <span className={`rounded-full px-2 py-0.5 text-[10px] ${statusTone.pill}`}>
                    {entry.status}
                  </span>
                </summary>
                {workerRef ? (
                  <div
                    className={`mt-1 rounded-xl rounded-tl-md px-2.5 py-2 text-[11px] whitespace-pre-wrap break-words ${statusTone.bubble}`}>
                    {workerRef.before}
                    <WorkerThreadRefCard
                      ref={workerRef.ref}
                      status={workerStatusFromEntry(entry.status)}
                    />
                    {workerRef.after ? <div className="mt-1">{workerRef.after}</div> : null}
                  </div>
                ) : formatted.detail ? (
                  <div
                    className={`mt-1 rounded-xl rounded-tl-md px-2.5 py-2 text-[11px] whitespace-pre-wrap break-words ${statusTone.bubble}`}>
                    {formatted.detail}
                  </div>
                ) : detailContent ? (
                  <pre
                    className={`mt-1 max-h-24 overflow-y-auto rounded px-2 py-1 font-mono text-[10px] whitespace-pre-wrap break-all ${statusTone.bubble} ${statusTone.code}`}>
                    {detailContent}
                  </pre>
                ) : null}
                {subagent ? <SubagentActivityBlock subagent={subagent} /> : null}
              </details>
            ) : (
              <div className="ml-1 flex items-center gap-2">
                <span className="font-medium text-stone-600 dark:text-neutral-300">
                  {formatted.title}
                </span>
                <span className={`rounded-full px-2 py-0.5 text-[10px] ${statusTone.pill}`}>
                  {entry.status}
                </span>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
