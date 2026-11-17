import createDebug from 'debug';
import { useState } from 'react';

import { ToolTimeline } from '../../../components/assistant-ui/elements/tool-timeline';
import {
  CollapsibleContent,
  CollapsibleRoot,
  CollapsibleTrigger,
} from '../../../components/ui/Collapsible';
import { useT } from '../../../lib/i18n/I18nContext';
import type { ProcessingTranscriptItem, ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import { formatTimelineEntry, stripToolCallEnvelopes } from '../../../utils/toolTimelineFormatting';
import { WorkerThreadRefCard } from '../components/WorkerThreadRefCard';
import { parseWorkerThreadRef } from '../utils/workerThreadRef';
import { ProcessingTranscript } from './processingTranscript';
import { SubagentActivityCard } from './SubagentActivityCard';
import {
  agentNameTone,
  AgentTimelineRow,
  coalesceTimelineEntries,
  normalizeToolBody,
  RepeatCount,
  workerStatusFromEntry,
} from './toolTimelineRowHelpers';

/** Tail of the parent's in-flight response shown in the processing panel. */
const RESPONSE_PREVIEW_CHARS = 320;

const log = createDebug('app:conversations:tool-timeline-adapter');

/**
 * The parent agent's live response, surfaced inside the processing panel while
 * the turn is in flight — its lead-in narration ("Let me check your Notion…")
 * belongs with the work it's narrating, not in a standalone chat bubble. The
 * final answer still lands in the message bubble once the turn settles.
 */
function LiveResponseBlock({ text }: { text: string }) {
  const { t } = useT();
  const clean = stripToolCallEnvelopes(text)
    .replace(/[ \t]+\n/g, '\n')
    .trimEnd();
  const shown = clean.slice(-RESPONSE_PREVIEW_CHARS);
  if (!shown.trim()) return null;
  return (
    <CollapsibleRoot
      defaultOpen
      data-testid="agent-live-response"
      className="group/resp mt-1.5 border-l-2 border-primary-300 pl-2 dark:border-primary-500/50">
      <CollapsibleTrigger
        size="sm"
        className="justify-start gap-1 px-0 py-0 hover:bg-transparent"
        aria-label={t('conversations.agentTaskInsights.response')}>
        <span aria-hidden className="text-[11px] leading-none">
          💬
        </span>
        <span className="text-[11px] font-semibold tracking-wide text-primary-500 uppercase dark:text-primary-300">
          {t('conversations.agentTaskInsights.response')}
        </span>
        <span
          aria-hidden
          className="text-[10px] text-content-faint transition-transform group-data-[state=open]:rotate-90">
          ▶
        </span>
      </CollapsibleTrigger>
      <CollapsibleContent size="sm" className="px-0 pb-0">
        <p className="mt-0.5 text-[12px] leading-snug wrap-break-word whitespace-pre-wrap text-content-secondary">
          {clean.length > RESPONSE_PREVIEW_CHARS ? (
            <span className="text-content-faint">…</span>
          ) : null}
          {shown}
          <span
            aria-hidden
            className="ml-0.5 inline-block h-3 w-1 animate-pulse bg-primary-400 align-middle"
          />
        </p>
      </CollapsibleContent>
    </CollapsibleRoot>
  );
}

/** Neutral surface tone for an expanded row's body. */
const BODY_SURFACE = 'bg-surface-muted';

/**
 * Height of the in-flight timeline viewport. While a turn is active the row
 * list is windowed to this height and auto-follows the newest activity.
 */
const TIMELINE_VIEWPORT_CLASS = 'max-h-64 overflow-y-auto overscroll-contain';

/** Distance from the bottom (px) still treated as "pinned to the live edge". */

/**
 * One expandable timeline row's disclosure. The row follows `autoExpand`
 * whenever THAT value changes (running → settled collapses it again), but a
 * manual toggle in between sticks until the next such change.
 */
function TimelineRowDisclosure({
  autoExpand,
  title,
  titleClassName,
  count,
  children,
}: {
  autoExpand: boolean;
  title: string;
  titleClassName: string;
  count: number;
  children: React.ReactNode;
}) {
  const [open, setOpen] = useState(autoExpand);
  const [prevAuto, setPrevAuto] = useState(autoExpand);
  if (prevAuto !== autoExpand) {
    setPrevAuto(autoExpand);
    setOpen(autoExpand);
  }
  return (
    <CollapsibleRoot
      open={open}
      onOpenChange={next => {
        log('timeline-row: user toggled open=%s (auto would be %s)', next, autoExpand);
        setOpen(next);
      }}
      className="group/row">
      <CollapsibleTrigger
        size="sm"
        className="justify-start gap-1.5 px-0 py-0 font-normal hover:bg-transparent">
        <span className={`text-[13px] font-medium ${titleClassName}`}>{title}</span>
        <RepeatCount count={count} />
        <span
          aria-hidden
          className="text-[11px] text-content-faint transition-transform group-data-[state=open]:rotate-90">
          ▶
        </span>
      </CollapsibleTrigger>
      <CollapsibleContent forceMount size="sm" className="px-0 pb-0">
        {children}
      </CollapsibleContent>
    </CollapsibleRoot>
  );
}

export interface ToolTimelineAdapterProps {
  entries: ToolTimelineEntry[];
  /** Compact chat mode: when set, a finished step renders as a single
   * `label + "View details →"` line (no inline expand) and the link opens the
   * side panel scoped to *that* step via this callback. */
  onViewDetails?: (entry: ToolTimelineEntry) => void;
  /** Opens the whole-run "Agent Process Source" panel. When set, a compact
   * "View full agent process Source →" link sits beside the group header. */
  onViewWholeRun?: () => void;
  /** Expand every row's details by default (used by the "Agent Process
   * Source" panel). In the inline chat only the latest running row auto-expands. */
  expandAllRows?: boolean;
  /** The parent agent's in-flight response text. */
  liveResponse?: string;
  /** Whether a turn is in flight on this thread's lifecycle. Falls back to
   * `isRunning` when omitted (correct for a settled/past-turn render). */
  turnActive?: boolean;
  /** The turn's interleaved processing transcript (narration + thinking + tool
   * pointers, in stream order). */
  transcript?: ProcessingTranscriptItem[];
}

/**
 * The agent-run timeline rendered above an assistant answer — the
 * "Agentic task insights" surface from the Figma Chat design — re-hosted on
 * the vendored `elements/tool-timeline` shell (`ToolTimeline`) instead of the
 * bespoke `<CollapsibleRoot>` + rail the deleted `ToolTimelineBlock` used.
 *
 * Also used by `AgentProcessSourcePanel` (the whole-run side panel) and
 * `FlowRunInspectorDrawer` (a flow run's tool parts).
 */
export function ToolTimelineAdapter({
  entries,
  onViewDetails,
  onViewWholeRun,
  expandAllRows = false,
  liveResponse,
  turnActive,
  transcript,
}: ToolTimelineAdapterProps) {
  const { t } = useT();

  // Sticky override for the outer "Agentic task insights" group: see the
  // deleted `ToolTimelineBlock` for the full history of this mechanic (#4942,
  // #5008). `null` means the user hasn't explicitly toggled it on THIS mount
  // yet, so the group falls back to the auto rule (open while running,
  // collapsed once settled).
  const [userOverrideOpen, setUserOverrideOpen] = useState<boolean | null>(null);

  const isRunning = entries.some(entry => entry.status === 'running');
  const settleSignal = turnActive ?? isRunning;

  const [prevSettleSignal, setPrevSettleSignal] = useState(settleSignal);
  if (prevSettleSignal !== settleSignal) {
    if (prevSettleSignal && !settleSignal) {
      log('agent-task-insights: turn settled (running→done), resetting user override');
      setUserOverrideOpen(null);
    }
    setPrevSettleSignal(settleSignal);
  }

  // ── In-flight viewport: fixed height + auto-follow ──────────────────────
  const windowed = turnActive === true && !expandAllRows;
  // Render whenever there is EITHER a tool row or transcript prose.
  if (entries.length === 0 && !(transcript && transcript.length > 0)) return null;

  const ordered = [...entries].sort((a, b) => a.seq - b.seq);
  const latestRunningEntryId = [...ordered].reverse().find(entry => entry.status === 'running')?.id;

  const wholeRunLink = onViewWholeRun ? (
    <button
      type="button"
      onClick={() => {
        log('agent-task-insights: opening whole-run process source');
        onViewWholeRun();
      }}
      data-testid="view-process-source"
      className="shrink-0 text-[11px] font-medium text-primary-600 hover:underline dark:text-primary-300">
      {t('conversations.agentTaskInsights.viewProcessSource')} →
    </button>
  ) : null;

  const rows = coalesceTimelineEntries(ordered);

  const body = (
    <>
      <div
        data-testid="tool-timeline-viewport"
        data-windowed={windowed ? 'true' : 'false'}
        className={windowed ? TIMELINE_VIEWPORT_CLASS : undefined}>
        {transcript && transcript.length > 0 ? (
          <ProcessingTranscript
            transcript={transcript}
            entries={ordered}
            live={turnActive ?? isRunning}
          />
        ) : (
          <div className="text-sm text-content-faint">
            {rows.map(({ entry, count }, index) => {
              const formatted = formatTimelineEntry(entry, t);
              const detailContent =
                normalizeToolBody(formatted.detail) ?? normalizeToolBody(entry.argsBuffer);
              const workerRef = parseWorkerThreadRef(formatted.detail ?? entry.detail);
              const subagent = entry.subagent;
              const resultContent = normalizeToolBody(entry.result);
              const expandable = detailContent != null || subagent != null || resultContent != null;
              const isLatestRunning =
                latestRunningEntryId != null && latestRunningEntryId === entry.id;
              const shouldAutoExpand = expandAllRows || isLatestRunning;
              const nameTone = agentNameTone(entry.status);
              const compact = onViewDetails != null && !isLatestRunning;

              return (
                <AgentTimelineRow
                  key={entry.id}
                  isFirst={index === 0}
                  isLast={index === rows.length - 1}>
                  {compact ? (
                    <div className="space-y-1">
                      <button
                        type="button"
                        onClick={() => onViewDetails(entry)}
                        data-testid="view-details"
                        className="group/details flex items-center gap-1.5 text-left">
                        <span
                          className={`text-[13px] font-medium ${nameTone.replace('animate-pulse ', '')} group-hover/details:underline`}>
                          {formatted.title}
                        </span>
                        <RepeatCount count={count} />
                        <span className="text-[13px] font-medium text-primary-600 dark:text-primary-300">
                          →
                        </span>
                      </button>
                      {resultContent && entry.status === 'error' ? (
                        <pre
                          data-testid="tool-result-output"
                          className={`max-h-40 overflow-y-auto rounded px-2 py-1 font-mono text-[12px] whitespace-pre-wrap break-all text-content-secondary ${BODY_SURFACE}`}>
                          {resultContent}
                        </pre>
                      ) : null}
                    </div>
                  ) : expandable ? (
                    <TimelineRowDisclosure
                      autoExpand={shouldAutoExpand}
                      title={formatted.title}
                      titleClassName={nameTone}
                      count={count}>
                      {workerRef ? (
                        <div
                          className={`mt-1 rounded-xl rounded-tl-md px-2.5 py-2 text-[13px] whitespace-pre-wrap wrap-break-word text-content-secondary ${BODY_SURFACE}`}>
                          {workerRef.before}
                          <WorkerThreadRefCard
                            ref={workerRef.ref}
                            status={workerStatusFromEntry(entry.status)}
                          />
                          {workerRef.after ? <div className="mt-1">{workerRef.after}</div> : null}
                        </div>
                      ) : formatted.detail ? (
                        <div
                          className={`mt-1 rounded-xl rounded-tl-md px-2.5 py-2 text-[13px] whitespace-pre-wrap wrap-break-word text-content-secondary ${BODY_SURFACE}`}>
                          {formatted.detail}
                        </div>
                      ) : detailContent ? (
                        <pre
                          className={`mt-1 max-h-24 overflow-y-auto rounded px-2 py-1 font-mono text-[12px] whitespace-pre-wrap break-all text-content-secondary ${BODY_SURFACE}`}>
                          {detailContent}
                        </pre>
                      ) : null}
                      {resultContent ? (
                        <pre
                          data-testid="tool-result-output"
                          className={`mt-1 max-h-40 overflow-y-auto rounded px-2 py-1 font-mono text-[12px] whitespace-pre-wrap break-all text-content-secondary ${BODY_SURFACE}`}>
                          {resultContent}
                        </pre>
                      ) : null}
                      {subagent ? <SubagentActivityCard activity={subagent} /> : null}
                    </TimelineRowDisclosure>
                  ) : (
                    <div className="flex items-center gap-1.5">
                      <span className={`text-[13px] font-medium ${nameTone}`}>
                        {formatted.title}
                      </span>
                      <RepeatCount count={count} />
                    </div>
                  )}
                </AgentTimelineRow>
              );
            })}
          </div>
        )}
      </div>
      {liveResponse ? <LiveResponseBlock text={liveResponse} /> : null}
    </>
  );

  const autoOpen = settleSignal || expandAllRows;
  const open = userOverrideOpen ?? autoOpen;
  const title = t('conversations.agentTaskInsights.title');

  return (
    <div className="group/insights mb-2 px-1 py-0">
      {wholeRunLink ? <div className="mb-1.5 flex items-center gap-1.5">{wholeRunLink}</div> : null}
      <ToolTimeline
        data-testid="agent-task-insights"
        streaming={false}
        restingLabel={title}
        activeLabel={title}
        open={open}
        onOpenChange={next => {
          log('agent-task-insights: user toggled open=%s (auto would be %s)', next, autoOpen);
          setUserOverrideOpen(next);
        }}>
        {body}
      </ToolTimeline>
    </div>
  );
}

export default ToolTimelineAdapter;
