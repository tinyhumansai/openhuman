import { ReasoningTraceText } from '@/components/assistant-ui/elements/reasoning-trace';
import type { ReasoningTiming } from '@/components/assistant-ui/elements/reasoningSteps';

import { useT } from '../../../lib/i18n/I18nContext';
import type {
  ProcessingTranscriptItem,
  ToolTimelineEntry,
  ToolTimelineEntryStatus,
} from '../../../store/chatRuntimeSlice';
import {
  buildProcessingBlocks,
  formatTimelineEntry,
  presentTimelineEntry,
  stripToolCallEnvelopes,
} from '../../../utils/toolTimelineFormatting';
import { ToolIcon } from '../tools/ToolIcon';
import { SubagentActivityCard } from './SubagentActivityCard';
import { ToolFailureCard } from './ToolFailureCard';

/**
 * The Hermes-style "View processing" body: the agent's narration and hidden
 * reasoning flow inline as prose, while runs of consecutive tool calls
 * collapse into a single group under a human summary ("Read 2 files"), each
 * step a sentence + a type icon, ending in a single "Done" check.
 *
 * Folded in from the deleted `components/ProcessingTranscriptView.tsx` as
 * part of the assistant-ui elements migration — used by
 * `ToolTimelineAdapter.tsx` (the inline rail / Agent Process Source panel's
 * whole-run view). Unlike the deleted component, sub-agent rows always render
 * through the vendored `elements/task-card` primitives (`SubagentActivityCard`)
 * rather than accepting a `renderSubagent` injection — both of this
 * component's remaining callers want exactly that renderer, so the seam that
 * used to avoid an import cycle is no longer needed.
 *
 * Falls back to a single tool group when no ordered transcript is present
 * (legacy snapshot), so older turns still show their steps.
 */
export function ProcessingTranscript({
  transcript,
  entries,
  live = false,
}: {
  transcript: ProcessingTranscriptItem[];
  entries: ToolTimelineEntry[];
  /**
   * True while the turn that produced `transcript` is still in flight. The
   * trailing thinking block then renders EXPANDED through
   * {@link LiveThinkingBlock} — a reasoning-tier model can spend the whole
   * time-to-first-token window streaming `thinking_delta`s and nothing else,
   * and a collapsed 💭 row hides the only evidence the agent is working. Once
   * the turn settles (or a later block lands) it becomes the quiet collapsed
   * block every other thought uses.
   */
  live?: boolean;
}) {
  const { t } = useT();
  const blocks = buildProcessingBlocks(transcript, entries, t);
  if (blocks.length === 0) return null;

  return (
    <div className="space-y-2.5" data-testid="processing-transcript">
      {blocks.map((block, index) => {
        if (block.kind === 'narration') {
          return (
            <p
              key={block.key}
              data-testid="processing-narration"
              className="text-[13px] leading-relaxed wrap-break-word whitespace-pre-wrap text-content-secondary">
              {block.text}
            </p>
          );
        }
        if (block.kind === 'thinking') {
          const timing =
            block.startedAt !== undefined || block.endedAt !== undefined
              ? { startedAt: block.startedAt, endedAt: block.endedAt }
              : undefined;
          return live && index === blocks.length - 1 ? (
            <LiveThinkingBlock key={block.key} text={block.text} timing={timing} />
          ) : (
            <ThinkingBlock key={block.key} text={block.text} timing={timing} />
          );
        }
        return <ToolGroupBlock key={block.key} summary={block.summary} entries={block.entries} />;
      })}
    </div>
  );
}

/**
 * The agent's reasoning, rendered through the shared static reasoning panel
 * in its non-collapsible form: the rail is the place the trail stays visible,
 * so a settled thought shows its "Thought for Ns" header and titled steps
 * inline rather than behind a disclosure.
 */
function ThinkingBlock({ text, timing }: { text: string; timing?: ReasoningTiming }) {
  const clean = stripToolCallEnvelopes(text).trim();
  if (!clean) return null;
  return (
    <ReasoningTraceText
      text={clean}
      timing={timing}
      streaming={false}
      collapsible={false}
      data-testid="processing-thinking"
    />
  );
}

/** The agent's reasoning while it is still streaming: the same static panel,
 *  live — the newest heading shimmers beside a ticking elapsed badge, and a
 *  long trace scrolls inside a bounded region pinned to its newest tokens,
 *  so the user sees the turn progressing during the window before any
 *  narration or tool call exists to show. */
function LiveThinkingBlock({ text, timing }: { text: string; timing?: ReasoningTiming }) {
  const clean = stripToolCallEnvelopes(text).trim();
  if (!clean) return null;
  return (
    <div aria-live="polite">
      <ReasoningTraceText
        text={clean}
        timing={timing}
        streaming
        collapsible={false}
        data-testid="processing-thinking-live"
      />
    </div>
  );
}

/** A collapsible group of consecutive tool rows under a human summary. */
function ToolGroupBlock({ summary, entries }: { summary: string; entries: ToolTimelineEntry[] }) {
  const { t } = useT();
  const allSettled = entries.every(e => e.status !== 'running');
  const anyError = entries.some(e => e.status === 'error');
  return (
    <details open className="group/group" data-testid="processing-tool-group">
      <summary className="flex cursor-pointer list-none items-center gap-1.5 select-none marker:hidden">
        <span className="text-[12px] font-medium text-content-secondary">{summary}</span>
        <span className="text-[9px] text-content-faint transition-transform group-open/group:rotate-90">
          ▶
        </span>
      </summary>
      <ul className="mt-1 ml-1 space-y-1 border-l border-line pl-3">
        {entries.map(entry => (
          <ToolRow key={entry.id} entry={entry} />
        ))}
        {allSettled ? (
          <li className="flex items-center gap-1.5 pt-0.5">
            <StatusGlyph status={anyError ? 'error' : 'success'} />
            <span className="text-[11px] text-content-faint">
              {t('conversations.agentTaskInsights.done')}
            </span>
          </li>
        ) : null}
      </ul>
    </details>
  );
}

/** One tool step: type icon + human sentence + contextual detail chip. */
function ToolRow({ entry }: { entry: ToolTimelineEntry }) {
  const { t } = useT();
  const { title, detail } = formatTimelineEntry(entry, t);
  return (
    <li className="flex flex-col gap-1" data-testid="processing-tool-row">
      <div className="flex items-start gap-1.5">
        <span className="mt-0.5 shrink-0 text-content-faint">
          <ToolIcon presentation={presentTimelineEntry(entry)} className="size-3" />
        </span>
        <span className="min-w-0 text-[12px] text-content-secondary">
          {title}
          {detail ? (
            <span className="ml-1 rounded bg-surface-subtle px-1 py-px font-mono text-[10px] text-content-muted">
              {detail}
            </span>
          ) : null}
          {entry.status === 'error' && entry.failure ? (
            <span className="mt-1 block">
              <ToolFailureCard
                toolName={entry.name}
                target={detail ?? title}
                failure={entry.failure}
                diagnostic={entry.result}
              />
            </span>
          ) : null}
        </span>
      </div>
      {/* A delegated sub-agent's own tool calls hang off the parent entry, so
          without this the whole child run collapsed into this single line.
          Rendered as a `<div>` SIBLING under the `<li>` (indented past the
          icon), not nested inside the label `<span>` — the delegation card
          renders a `<div>`, and `<div>`-inside-`<span>` is invalid nesting. */}
      {entry.subagent ? (
        <div className="ml-5" data-testid="processing-subagent">
          <SubagentActivityCard activity={entry.subagent} />
        </div>
      ) : null}
    </li>
  );
}

/** Compact terminal status glyph for the group's "Done" line. */
function StatusGlyph({ status }: { status: ToolTimelineEntryStatus }) {
  if (status === 'error') {
    return <span className="text-[11px] text-coral-600 dark:text-coral-300">✕</span>;
  }
  return <span className="text-[11px] text-sage-600 dark:text-sage-300">✓</span>;
}
