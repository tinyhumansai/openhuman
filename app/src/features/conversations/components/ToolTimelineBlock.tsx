import createDebug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import {
  CollapsibleContent,
  CollapsibleRoot,
  CollapsibleTrigger,
} from '../../../components/ui/Collapsible';
import { useT } from '../../../lib/i18n/I18nContext';
import type {
  ProcessingTranscriptItem,
  SubagentActivity,
  ToolTimelineEntry,
} from '../../../store/chatRuntimeSlice';
import { formatTimelineEntry, stripToolCallEnvelopes } from '../../../utils/toolTimelineFormatting';
import { parseWorkerThreadRef } from '../utils/workerThreadRef';
import { agentNameTone, AgentTimelineRail } from './AgentTimelineRail';
import { AssistantUiSubagentCall, isActiveSubagentStatus } from './AssistantUiSubagentCall';
import { ProcessingTranscriptView } from './ProcessingTranscriptView';
import {
  coalesceTimelineEntries,
  normalizeToolBody,
  RepeatCount,
  workerStatusFromEntry,
} from './toolTimelineRows';
import { WorkerThreadRefCard } from './WorkerThreadRefCard';

/** Tail of the parent's in-flight response shown in the processing panel. */
const RESPONSE_PREVIEW_CHARS = 320;

const log = createDebug('app:conversations:tool-timeline');

/**
 * The parent agent's live response, surfaced inside the processing panel while
 * the turn is in flight — its lead-in narration ("Let me check your Notion…")
 * belongs with the work it's narrating, not in a standalone chat bubble. The
 * final answer still lands in the message bubble once the turn settles.
 * Collapsible + accented apart from the stone-toned sub-agent Thoughts so the
 * parent's own voice reads as the primary thread.
 *
 * The disclosure is the shared Radix {@link CollapsibleRoot}, which gives the
 * trigger real `aria-expanded` / `aria-controls` wiring that the hand-rolled
 * `<details>`/`<summary>` pair never had.
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

/**
 * Neutral surface tones for an expanded row's body (worker-thread card,
 * detail bubble, code block). Per the Figma "Agentic task insights"
 * design these read as plain light cards rather than status-coloured
 * panels — the row's *status* is conveyed by the agent name (see
 * {@link agentNameTone}), so the body stays visually quiet.
 */
const BODY_SURFACE = 'bg-surface-muted';

/**
 * Height of the in-flight timeline viewport. While a turn is active the row
 * list is windowed to this height and auto-follows the newest activity, so a
 * long run (dozens of steps, each able to expand) can no longer grow without
 * bound and shove the composer + message list around mid-turn. Settled turns
 * are untouched — they still collapse to the group summary as before.
 */
const TIMELINE_VIEWPORT_CLASS = 'max-h-64 overflow-y-auto overscroll-contain';

/** Distance from the bottom (px) still treated as "pinned to the live edge". */
const STICK_TO_BOTTOM_SLACK_PX = 24;

/**
 * One expandable timeline row's disclosure.
 *
 * Replaces the previous `<details open={autoExpand}>`, whose semantics this
 * reproduces exactly rather than approximately: the row follows `autoExpand`
 * whenever THAT value changes (running → settled collapses it again), but a
 * manual toggle in between sticks until the next such change. A plain
 * `defaultOpen` would have dropped the first half; a fully controlled
 * `open={autoExpand}` would have dropped the second.
 *
 * `forceMount` keeps the body in the DOM while collapsed, which is what
 * `<details>` did — the rows are never "wiped", only hidden.
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
  // Render-time adjustment (React's documented "reset state on prop change"
  // pattern) rather than an effect, so the corrected frame is the first one
  // painted.
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

/**
 * The agent-run timeline rendered above an assistant answer — the
 * "Agentic task insights" surface from the Figma Chat design.
 *
 * Each {@link ToolTimelineEntry} is a row on a shared vertical timeline
 * rail ({@link AgentTimelineRail}); the agent name carries the run state
 * (pulsing while in flight, solid when done) and expands in place to show
 * its detail/code/sub-agent activity. The whole group sits under a
 * collapsible "Agentic task insights" header so the user can fold the live
 * activity away.
 */
export function ToolTimelineBlock({
  entries,
  onViewSubagent,
  onViewDetails,
  onViewWholeRun,
  expandAllRows = false,
  liveResponse,
  turnActive,
  transcript,
}: {
  entries: ToolTimelineEntry[];
  /** Opens the full-transcript drawer for a subagent row. When omitted,
   * subagent cards render without the "view full processing" affordance
   * (e.g. interrupted-snapshot rendering with no live driver). */
  onViewSubagent?: (subagent: SubagentActivity) => void;
  /** Compact chat mode: when set, a finished step renders as a single
   * `label + "View details →"` line (no inline expand) and the link opens the
   * side panel scoped to *that* step via this callback. The panel itself
   * renders without `onViewDetails` to keep the full expanded view. */
  onViewDetails?: (entry: ToolTimelineEntry) => void;
  /** Opens the whole-run "Agent Process Source" panel. When set, a compact
   * "View full agent process Source →" link sits in the group header beside the
   * "Agentic task insights" title (clicking it does NOT toggle the collapse). */
  onViewWholeRun?: () => void;
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
  /** Whether a turn is in flight on this thread's lifecycle
   * (`inferenceTurnLifecycleByThread`), the same signal the chat threads page
   * uses. When provided, the sticky `userOverrideOpen` reset fires on THIS
   * value's true→false edge — once per USER TURN — instead of on `isRunning`'s
   * edge, which flips once PER SUB-AGENT within a single turn (each
   * subagent spawn→settle) and made the panel flicker open/closed as
   * sub-agents ran (regression from #5008). Falls back to `isRunning` when
   * omitted, which is correct for a settled/past-turn render (there is no
   * turn left to track). */
  turnActive?: boolean;
  /** The turn's interleaved processing transcript (narration + thinking + tool
   * pointers, in stream order). When non-empty the rail renders it through
   * {@link ProcessingTranscriptView} — the SAME component the Agent Process
   * Source panel uses — so the agent's prose and its tool steps appear in one
   * surface instead of narration living in the chat stream and thinking in a
   * separate bubble. Omitted/empty falls back to the tool-row list, which is
   * still correct for legacy snapshots that predate the transcript. */
  transcript?: ProcessingTranscriptItem[];
}) {
  const { t } = useT();

  // Sticky override for the outer "Agentic task insights" group: `null` means
  // the user hasn't explicitly toggled it on THIS mount yet, so the group
  // falls back to the auto rule below (open while running, collapsed once
  // settled). Once the user clicks the trigger, this pins their choice —
  // including across later turns that stream onto the SAME mounted block
  // (e.g. the workflow copilot's dedicated thread, whose `ToolTimelineBlock`
  // stays mounted for the life of the conversation while `entries` keeps
  // growing turn over turn) — so a new turn's activity landing no longer
  // involuntarily re-collapses (or re-expands) a choice the user already
  // made ("Agentic task insights keeps collapsing on every new feedback").
  // Deliberately component-local state, not lifted to Redux:
  // the block never remounts mid-conversation in the one place this bug was
  // reported (`WorkflowCopilotPanel` renders it at a stable JSX position
  // with entries accumulating in `toolTimelineByThread`, not reset per
  // turn), so a plain `useState` already survives every turn it needs to.
  const [userOverrideOpen, setUserOverrideOpen] = useState<boolean | null>(null);

  // Whether *any* entry is currently running — computed here (ahead of the
  // `entries.length === 0` early return below) purely so the render-time
  // reset adjustment that follows runs every render; order doesn't matter for
  // this existence check, unlike `latestRunningEntryId` further down, which
  // needs the seq-sorted order to pick a specific "latest" row.
  const isRunning = entries.some(entry => entry.status === 'running');

  // The signal the reset below watches for a "turn just settled" edge. Prefer
  // the real TURN lifecycle (`turnActive`, sourced from
  // `inferenceTurnLifecycleByThread` — the same signal the chat threads page
  // uses) when the caller supplies it: it flips true→false exactly once per
  // USER TURN. `isRunning` is only a fallback for callers with no turn
  // lifecycle to hand (e.g. a settled/past-turn render, where entries never
  // change again anyway) — used directly it flips once PER SUB-AGENT within a
  // single turn (each subagent spawn→settle), which reset the override (and
  // so auto-collapsed the panel) repeatedly within one turn and made it
  // flicker open/closed as sub-agents ran (#5008 regression).
  const settleSignal = turnActive ?? isRunning;

  // Reset the user's manual open/close override on the settleSignal's
  // true→false edge (a turn just finished) so the auto-collapse applies to
  // the just-settled turn. The override only sticks WITHIN a turn —
  // preventing involuntary mid-feedback collapse (#4942) — not permanently
  // across turns.
  //
  // Done as a render-time adjustment (comparing against `prevSettleSignal`
  // state and calling both setters synchronously in the render body), not a
  // `useEffect`, per React's documented pattern for resetting state on a prop
  // transition: it bails out and re-renders with the reset applied before
  // paint, instead of committing a stale (still-collapsed/expanded) frame
  // and only correcting it a tick later once the effect runs.
  const [prevSettleSignal, setPrevSettleSignal] = useState(settleSignal);
  if (prevSettleSignal !== settleSignal) {
    if (prevSettleSignal && !settleSignal) {
      log('agent-task-insights: turn settled (running→done), resetting user override');
      setUserOverrideOpen(null);
    }
    setPrevSettleSignal(settleSignal);
  }

  // ── In-flight viewport: fixed height + auto-follow ──────────────────────
  // Windowed ONLY while the turn is in flight, and never under
  // `expandAllRows` (the Agent Process Source panel wants the full list, not
  // a 16rem porthole).
  //
  // Keyed off `turnActive` directly rather than `settleSignal` — the latter
  // falls back to `isRunning`, which flips once per SUB-AGENT inside a single
  // turn and would re-window + re-pin the viewport repeatedly mid-turn (the
  // same failure class as the #5008 collapse flicker). Callers that pass no
  // `turnActive` (settled/past-turn renders) never window at all, so nothing
  // about historical turns changes.
  const windowed = turnActive === true && !expandAllRows;
  const viewportRef = useRef<HTMLDivElement | null>(null);
  // Whether to keep pinning to the newest activity. A plain ref, not state:
  // no render output depends on it, and mutating it from the scroll handler
  // must not trigger a re-render on every wheel tick.
  const followTailRef = useRef(true);

  // Re-pin whenever a new turn starts windowing, so a user who scrolled up
  // during the previous turn isn't stuck detached for the next one.
  useEffect(() => {
    if (windowed) followTailRef.current = true;
  }, [windowed]);

  // Detach on scroll-up so reading an earlier step doesn't get yanked back
  // down by the next tool event; re-attach when they return to the bottom.
  const handleViewportScroll = () => {
    const el = viewportRef.current;
    if (!el) return;
    followTailRef.current =
      el.scrollHeight - el.scrollTop - el.clientHeight <= STICK_TO_BOTTOM_SLACK_PX;
  };

  // Follow the live edge. A ResizeObserver on the row list (rather than an
  // effect keyed on row count) catches every way the content grows: a new row
  // arriving, the running row auto-expanding, and `tool_args_delta` streaming
  // into an already-expanded row — the last of which changes no React key at
  // all and would otherwise silently stop following mid-tool.
  //
  // Attached via a CALLBACK REF, not a `useEffect([windowed])`. The effect
  // version never attached in a real turn: `windowed` flips true at the START
  // of the turn, when there is nothing to show yet, so the component returned
  // null, `viewportRef.current` was null, and the effect bailed. Rows arriving
  // afterwards re-rendered the viewport but did not change `windowed`, so the
  // effect never re-ran and no observer was ever created. A callback ref fires
  // whenever the node itself mounts or changes, which is exactly the event we
  // care about, and is immune to that ordering entirely.
  const observerRef = useRef<ResizeObserver | null>(null);
  const windowedRef = useRef(windowed);
  windowedRef.current = windowed;
  const attachViewport = useCallback((node: HTMLDivElement | null) => {
    observerRef.current?.disconnect();
    observerRef.current = null;
    viewportRef.current = node;
    if (!node || typeof ResizeObserver === 'undefined') return;
    // Children are in the DOM by the time a parent's ref callback runs.
    const inner = node.firstElementChild;
    if (!inner) return;
    const observer = new ResizeObserver(() => {
      // Read the live values through refs so the observer survives a settle →
      // re-arm without being torn down and rebuilt.
      if (!windowedRef.current || !followTailRef.current) return;
      // `auto`, not `smooth`: streaming deltas fire these back-to-back, and
      // queued smooth scrolls visibly lag behind the content.
      node.scrollTop = node.scrollHeight;
    });
    observer.observe(inner);
    observerRef.current = observer;
  }, []);
  useEffect(() => () => observerRef.current?.disconnect(), []);

  // Render whenever there is EITHER a tool row or transcript prose. Gating on
  // `entries` alone blanked the rail for the opening stretch of every turn —
  // narration streams before the first tool call — and hid a tool-less turn
  // (pure reasoning/narration) completely.
  if (entries.length === 0 && !(transcript && transcript.length > 0)) return null;

  // The rows + the parent's streaming response — shared by both the collapsible
  // (in-flight) and static (settled) header layouts below.
  // Sort by issue order (`seq`), not arrival order: a `tool_args_delta` for a
  // later parallel call can reach the store before an earlier call's own
  // event, which would otherwise create rows in the wrong order.
  // Sort a copy — `entries` may be a state slice other callers still rely on.
  const ordered = [...entries].sort((a, b) => a.seq - b.seq);
  // "Latest running" must be derived from the same seq-ordered list the rows
  // render from — not raw arrival order — or a running row that arrived late
  // but sorts earlier (e.g. seq [2, 0, 1]) gets treated as "latest" and the
  // wrong step stays expanded/linked in compact chat mode.
  const latestRunningEntryId = [...ordered].reverse().find(entry => entry.status === 'running')?.id;

  // Whole-run "View full agent process Source →" link — a SIBLING of the
  // disclosure trigger, not a child of it. Nesting one button inside another
  // is invalid HTML; as siblings the link needs no `stopPropagation` to avoid
  // toggling the group, and it stays outside the collapsible body so it is
  // reachable while the group is collapsed.
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

  // Coalesce runs of identical, body-less rows (e.g. a retry loop that spawns
  // the same integrations step 25×) into single `×N` rows before rendering.
  const rows = coalesceTimelineEntries(ordered);

  const body = (
    <>
      {/* Viewport wrapper. Stays in the tree in both modes so the row list is
          never remounted (and its disclosure state never reset) when a turn
          settles — only the height/scroll classes toggle. */}
      <div
        ref={attachViewport}
        onScroll={windowed ? handleViewportScroll : undefined}
        data-testid="tool-timeline-viewport"
        data-windowed={windowed ? 'true' : 'false'}
        className={windowed ? TIMELINE_VIEWPORT_CLASS : undefined}>
        {transcript && transcript.length > 0 ? (
          <ProcessingTranscriptView
            transcript={transcript}
            entries={ordered}
            renderSubagent={subagent => (
              <AssistantUiSubagentCall
                activity={subagent}
                running={isActiveSubagentStatus(subagent.status)}
                onView={onViewSubagent ? () => onViewSubagent(subagent) : undefined}
              />
            )}
          />
        ) : (
          <div className="text-sm text-content-faint">
            {rows.map(({ entry, count }, index) => {
              const formatted = formatTimelineEntry(entry);
              const detailContent =
                normalizeToolBody(formatted.detail) ?? normalizeToolBody(entry.argsBuffer);
              const workerRef = parseWorkerThreadRef(formatted.detail ?? entry.detail);
              const subagent = entry.subagent;
              const resultContent = normalizeToolBody(entry.result);
              // A subagent row should always render the expandable details so
              // its live activity is visible — even when there is no prompt
              // detail to show. Mirrors the rule that a non-subagent row only
              // expands when it has detail content (or a result to show).
              const expandable = detailContent != null || subagent != null || resultContent != null;
              const isLatestRunning =
                latestRunningEntryId != null && latestRunningEntryId === entry.id;
              const shouldAutoExpand = expandAllRows || isLatestRunning;
              const nameTone = agentNameTone(entry.status);
              // Chat mode: the currently-running step stays expanded inline in the
              // main UI; finished steps collapse to a compact "View details →" link
              // (their full activity lives in the side panel).
              const compact = onViewDetails != null && !isLatestRunning;

              return (
                <AgentTimelineRail
                  key={entry.id}
                  isFirst={index === 0}
                  isLast={index === rows.length - 1}>
                  {compact ? (
                    // Collapsed step: the whole label is the link — "Run Code →"
                    // opens the full-run panel scoped to this step. A collapsed row
                    // is backgrounded, so it never pulses — only the single active
                    // (expanded) step blinks. Strip `animate-pulse` from the tone.
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
                      {/* Output stays inline for FAILED steps only. On a success
                        the agent's final answer is already the compression of
                        what the tool returned, so repeating the raw result here
                        just duplicates it — and a multi-tool turn stacked a
                        scrollable <pre> per step above the answer. A failure is
                        the case where the answer is least trustworthy (or may
                        not mention the failure at all), so the evidence earns
                        its space. Successful output is still one click away via
                        this row's "→" and "View full agent process Source", and
                        expanded rows / the process panel are unchanged. */}
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
                        // What the tool returned (size-capped upstream). Scrolls
                        // inside its own box so a long result never floods the
                        // timeline.
                        <pre
                          data-testid="tool-result-output"
                          className={`mt-1 max-h-40 overflow-y-auto rounded px-2 py-1 font-mono text-[12px] whitespace-pre-wrap break-all text-content-secondary ${BODY_SURFACE}`}>
                          {resultContent}
                        </pre>
                      ) : null}
                      {subagent ? (
                        <AssistantUiSubagentCall
                          activity={subagent}
                          running={entry.status === 'running' || entry.status === 'awaiting_user'}
                          onView={onViewSubagent ? () => onViewSubagent(subagent) : undefined}
                        />
                      ) : null}
                    </TimelineRowDisclosure>
                  ) : (
                    <div className="flex items-center gap-1.5">
                      <span className={`text-[13px] font-medium ${nameTone}`}>
                        {formatted.title}
                      </span>
                      <RepeatCount count={count} />
                    </div>
                  )}
                </AgentTimelineRail>
              );
            })}
          </div>
        )}
      </div>
      {liveResponse ? <LiveResponseBlock text={liveResponse} /> : null}
    </>
  );

  // The group header is a static section label — the live "working" state is
  // conveyed by the pulsing agent-name rows, so it never repeats a "Working…"
  // string. Absent a user override, the group is auto-driven: open while the
  // run is in flight so the live activity is visible; collapsed once it
  // settles so a finished run (which can be dozens of steps) never dominates
  // the conversation. The full "Agent Process Source" panel forces every row
  // open via `expandAllRows`. Once the user has explicitly toggled the group,
  // THAT choice wins over the auto rule — otherwise a new turn streaming onto
  // an already-mounted block (settling, or starting a fresh run) would
  // silently flip `open` out from under the user's manual choice on every
  // turn.
  //
  // Driven by `settleSignal` (i.e. `turnActive` when the caller supplies it),
  // NOT by `isRunning`. `isRunning` means "a tool is executing *this instant*",
  // which goes false in every gap BETWEEN tools — while the agent reasons about
  // a result before issuing the next call. Keyed off that, the group snapped
  // shut a beat after each tool result and reopened when the next call started,
  // so a multi-tool turn flickered and a just-delivered result looked like it
  // had been wiped. #5008 already established `turnActive` as the correct
  // whole-turn signal and applied it to the override reset above; `autoOpen`
  // was left behind on `isRunning`. Same signal now drives both, so the group
  // stays open for the WHOLE turn and collapses once, at settle.
  const autoOpen = settleSignal || expandAllRows;
  const open = userOverrideOpen ?? autoOpen;

  return (
    // Radix `Collapsible`, fully controlled off `open` above: one source of
    // truth, plus the `aria-expanded`/`aria-controls` wiring the hand-rolled
    // `<details>`/`<summary>` pair never had. `forceMount` on the content keeps
    // the rows in the DOM while collapsed, exactly as `<details>` did — the
    // activity is hidden, never wiped.
    <CollapsibleRoot
      open={open}
      onOpenChange={next => {
        log('agent-task-insights: user toggled open=%s (auto would be %s)', next, autoOpen);
        setUserOverrideOpen(next);
      }}
      className="group/insights mb-2 px-1 py-0"
      data-testid="agent-task-insights">
      <div className="mb-1.5 flex items-center gap-1.5">
        <CollapsibleTrigger
          size="sm"
          className="w-auto justify-start gap-1.5 px-0 py-0 font-normal hover:bg-transparent">
          <span className="text-[13px] font-medium text-content-muted">
            {t('conversations.agentTaskInsights.title')}
          </span>
          <span
            aria-hidden
            className="text-[11px] text-content-faint transition-transform group-data-[state=open]:rotate-90">
            ▶
          </span>
        </CollapsibleTrigger>
        {wholeRunLink}
      </div>
      <CollapsibleContent forceMount size="sm" className="px-0 pb-0">
        {body}
      </CollapsibleContent>
    </CollapsibleRoot>
  );
}
