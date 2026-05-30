import { type ReactNode, useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { threadApi } from '../../../services/api/threadApi';
import type {
  SubagentActivity,
  SubagentTranscriptItem,
  ToolTimelineEntryStatus,
} from '../../../store/chatRuntimeSlice';
import type { ThreadMessage } from '../../../types/thread';
import { BubbleMarkdown } from './AgentMessageBubble';

/**
 * Rebuild a renderable transcript from a worker sub-thread's persisted
 * messages so a delegation can be reopened from memory after its live
 * stream is gone (navigation / cold boot). The first `user` message is the
 * parent's delegation prompt; `agent` messages with a `tool_name` in their
 * metadata are tool calls, the rest are the sub-agent's visible text.
 * Streamed reasoning isn't persisted, so reopened transcripts omit it.
 */
function transcriptFromMessages(messages: ThreadMessage[]): {
  prompt?: string;
  items: SubagentTranscriptItem[];
} {
  let prompt: string | undefined;
  const items: SubagentTranscriptItem[] = [];
  for (const m of messages) {
    const meta = m.extraMetadata ?? {};
    const iteration = typeof meta.iteration === 'number' ? meta.iteration : undefined;
    if (m.sender === 'user') {
      if (prompt === undefined) prompt = m.content;
      continue;
    }
    const toolName = typeof meta.tool_name === 'string' ? meta.tool_name : undefined;
    if (toolName) {
      items.push({ kind: 'tool', iteration, callId: m.id, toolName, status: 'success' });
    } else if (m.content.trim().length > 0) {
      items.push({ kind: 'text', iteration, text: m.content });
    }
  }
  return { prompt, items };
}

/**
 * Map a subagent row's terminal/running status to the visual tone used
 * across the drawer (header dot, status pill). Mirrors the colour
 * language of `ToolTimelineBlock` so the inline card and the drawer read
 * as the same surface.
 */
function statusTone(status: ToolTimelineEntryStatus | undefined): {
  dot: string;
  pill: string;
  label: 'statusRunning' | 'statusCompleted' | 'statusFailed';
} {
  if (status === 'success') {
    return {
      dot: 'bg-sage-500',
      pill: 'bg-sage-100 dark:bg-sage-500/20 text-sage-700 dark:text-sage-300',
      label: 'statusCompleted',
    };
  }
  if (status === 'error') {
    return {
      dot: 'bg-coral-500',
      pill: 'bg-coral-100 dark:bg-coral-500/20 text-coral-700 dark:text-coral-300',
      label: 'statusFailed',
    };
  }
  return {
    dot: 'bg-amber-500 animate-pulse',
    pill: 'bg-amber-100 dark:bg-amber-500/20 text-amber-700 dark:text-amber-300',
    label: 'statusRunning',
  };
}

function formatElapsed(ms: number): string {
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`;
}

/**
 * Full live-transcript view for one sub-agent, slid in from the right.
 *
 * Driven entirely off the live [`SubagentActivity`] the caller passes —
 * because the caller re-derives that object from Redux on every render,
 * the drawer updates token-by-token as `subagent_text_delta` /
 * `subagent_thinking_delta` events stream in. Shows the streamed
 * reasoning (collapsible), the streamed visible output (rendered as
 * Markdown), and the chronological list of child tool calls with their
 * status and timings.
 *
 * Rendered as `null` when no subagent is selected, so the parent can
 * mount it unconditionally and just flip `subagent`.
 */
export function SubagentDrawer({
  subagent,
  status,
  onClose,
}: {
  subagent: SubagentActivity | null;
  /** Lifecycle status of the owning timeline row (running/success/error). */
  status?: ToolTimelineEntryStatus;
  onClose: () => void;
}) {
  const { t } = useT();

  // Close on Escape for keyboard parity with the backdrop click.
  useEffect(() => {
    if (!subagent) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [subagent, onClose]);

  // Reopen-from-memory: when there's no live transcript (the row was
  // restored from a snapshot, or the user navigated back after the turn
  // ended) but a worker sub-thread backs it, load that thread's persisted
  // messages and render them as the conversation. Failures fall back to the
  // empty/working placeholder rather than blocking the drawer.
  // Tagged with the worker thread it was fetched for, so a pending request
  // for a previous thread can't paint the wrong conversation after the user
  // switches subagents.
  const [fetched, setFetched] = useState<{
    workerThreadId: string;
    prompt?: string;
    items: SubagentTranscriptItem[];
  } | null>(null);
  const liveTranscript = subagent?.transcript ?? [];
  const workerThreadId = subagent?.workerThreadId;
  const needsFetch = Boolean(subagent && workerThreadId && liveTranscript.length === 0);

  useEffect(() => {
    if (!needsFetch || !workerThreadId) {
      setFetched(null);
      return;
    }
    // Clear any prior thread's transcript up front so it can't linger while
    // the new request is in flight.
    setFetched(null);
    let cancelled = false;
    void threadApi
      .getThreadMessages(workerThreadId)
      .then(data => {
        if (!cancelled) setFetched({ workerThreadId, ...transcriptFromMessages(data.messages) });
      })
      .catch(() => {
        if (!cancelled) setFetched(null);
      });
    return () => {
      cancelled = true;
    };
  }, [needsFetch, workerThreadId]);

  if (!subagent) return null;

  const tone = statusTone(status);
  const isRunning = status !== 'success' && status !== 'error';
  // Only trust the fetched transcript when it belongs to the current worker.
  const fetchedForCurrent =
    fetched && workerThreadId && fetched.workerThreadId === workerThreadId ? fetched : null;
  const transcript = liveTranscript.length > 0 ? liveTranscript : (fetchedForCurrent?.items ?? []);
  const promptText = subagent.prompt ?? fetchedForCurrent?.prompt;
  // The last visible-text item gets the live cursor while the run is in
  // flight (the model is mid-sentence on its final/visible output).
  let lastTextIdx = -1;
  for (let i = transcript.length - 1; i >= 0; i -= 1) {
    if (transcript[i].kind === 'text') {
      lastTextIdx = i;
      break;
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex justify-end" data-testid="subagent-drawer">
      {/* Backdrop */}
      <button
        type="button"
        aria-label={t('conversations.subagent.close')}
        className="absolute inset-0 bg-stone-900/30 dark:bg-black/50"
        onClick={onClose}
      />
      <aside className="relative flex h-full w-full max-w-md flex-col bg-white dark:bg-neutral-900 shadow-xl">
        {/* Header */}
        <header className="flex items-center gap-2.5 border-b border-stone-200 dark:border-neutral-800 px-4 py-3">
          <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary-50 dark:bg-primary-500/15 text-base">
            🤖
          </span>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="truncate font-semibold text-stone-800 dark:text-neutral-100">
                {subagent.agentId}
              </span>
              <span className={`h-2 w-2 shrink-0 rounded-full ${tone.dot}`} />
            </div>
            <div className="flex flex-wrap items-center gap-1.5 text-[11px] text-stone-500 dark:text-neutral-400">
              <span className={`rounded-full px-1.5 py-0.5 ${tone.pill}`}>
                {t(`conversations.subagent.${tone.label}`)}
              </span>
              {subagent.childIteration != null && subagent.childMaxIterations != null ? (
                <span>
                  {t('conversations.toolTimeline.turn')} {subagent.childIteration}/
                  {subagent.childMaxIterations}
                </span>
              ) : subagent.iterations != null ? (
                <span>
                  {subagent.iterations} {t('conversations.toolTimeline.turn')}
                </span>
              ) : null}
              {subagent.elapsedMs != null ? <span>{formatElapsed(subagent.elapsedMs)}</span> : null}
              {subagent.mode ? <span>{subagent.mode}</span> : null}
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label={t('conversations.subagent.close')}
            className="shrink-0 rounded-full p-1.5 text-stone-400 hover:bg-stone-100 dark:hover:bg-neutral-800 hover:text-stone-600 dark:hover:text-neutral-200">
            ✕
          </button>
        </header>

        {/* Body — a parent↔subagent conversation: the parent's delegation
            prompt opens it, then the sub-agent replies as one chronological
            transcript (thinking, the text it produced, the tool calls that
            text triggered, the next turn — exactly as it was emitted). */}
        <div className="flex-1 space-y-3 overflow-y-auto px-4 py-4">
          {/* Parent → sub-agent: the delegation prompt (the "input"). */}
          {promptText ? (
            <div className="flex justify-end" data-testid="subagent-parent-prompt">
              <div className="max-w-[85%] rounded-2xl rounded-br-md bg-primary-500 px-3 py-2 text-sm text-white">
                <div className="mb-0.5 text-[10px] font-semibold uppercase tracking-wide text-white/70">
                  {t('conversations.subagent.parent')}
                </div>
                <div className="whitespace-pre-wrap break-words">{promptText}</div>
              </div>
            </div>
          ) : null}

          {/* Sub-agent side: avatar label + its turns. */}
          <div className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-stone-400 dark:text-neutral-500">
            <span>🤖</span>
            {subagent.agentId}
          </div>

          {transcript.length === 0 ? (
            <p className="text-xs italic text-stone-400 dark:text-neutral-500">
              {isRunning
                ? t('conversations.subagent.working')
                : t('conversations.subagent.noOutputYet')}
            </p>
          ) : (
            <ol className="space-y-2">
              {transcript.map((item, idx) => {
                // Insert a "Turn N" divider when the iteration advances.
                const prevIteration = idx > 0 ? transcript[idx - 1].iteration : undefined;
                const showTurn = item.iteration != null && item.iteration !== prevIteration;
                const turnDivider = showTurn ? (
                  <li
                    aria-hidden
                    className="flex items-center gap-2 pt-1 text-[10px] font-medium uppercase tracking-wide text-stone-400 dark:text-neutral-500"
                    data-testid="subagent-turn-divider">
                    <span className="h-px flex-1 bg-stone-200 dark:bg-neutral-800" />
                    {t('conversations.toolTimeline.turn')} {item.iteration}
                    <span className="h-px flex-1 bg-stone-200 dark:bg-neutral-800" />
                  </li>
                ) : null;

                if (item.kind === 'thinking') {
                  return (
                    <ItemWrapper key={`th-${idx}`} divider={turnDivider}>
                      <div
                        className="rounded-lg bg-stone-50 dark:bg-neutral-800/60 px-3 py-2"
                        data-testid="subagent-transcript-thinking">
                        <div className="mb-1 flex items-center gap-1.5 text-[11px] font-semibold text-stone-500 dark:text-neutral-400">
                          <span className="inline-block h-1.5 w-1.5 rounded-full bg-primary-400" />
                          {t('conversations.subagent.thinking')}
                        </div>
                        <pre className="whitespace-pre-wrap break-words font-sans text-[12px] leading-relaxed text-stone-600 dark:text-neutral-300">
                          {item.text}
                        </pre>
                      </div>
                    </ItemWrapper>
                  );
                }

                if (item.kind === 'text') {
                  return (
                    <ItemWrapper key={`tx-${idx}`} divider={turnDivider}>
                      <div data-testid="subagent-transcript-text">
                        <BubbleMarkdown content={item.text} />
                        {isRunning && idx === lastTextIdx ? (
                          <span className="ml-0.5 inline-block h-3 w-1 animate-pulse bg-primary-400 align-middle" />
                        ) : null}
                      </div>
                    </ItemWrapper>
                  );
                }

                const callTone =
                  item.status === 'running'
                    ? 'text-amber-700 dark:text-amber-300'
                    : item.status === 'success'
                      ? 'text-sage-700 dark:text-sage-300'
                      : 'text-coral-700 dark:text-coral-300';
                const statusLabel =
                  item.status === 'running'
                    ? t('conversations.subagent.statusRunning')
                    : item.status === 'success'
                      ? t('conversations.subagent.statusCompleted')
                      : t('conversations.subagent.statusFailed');
                return (
                  <ItemWrapper key={`tl-${item.callId}`} divider={turnDivider}>
                    <div
                      className="flex items-center gap-2 rounded-md border border-stone-200 bg-stone-50 px-2.5 py-1.5 text-xs dark:border-neutral-800 dark:bg-neutral-800/60"
                      data-testid="subagent-drawer-tool-call">
                      <span className={callTone}>🔧</span>
                      <span className="font-mono text-stone-700 dark:text-neutral-200">
                        {item.toolName}
                      </span>
                      <span className={`ml-auto ${callTone}`}>{statusLabel}</span>
                      {item.elapsedMs != null && item.status !== 'running' ? (
                        <span className="text-[10px] text-stone-400 dark:text-neutral-500">
                          {formatElapsed(item.elapsedMs)}
                        </span>
                      ) : null}
                    </div>
                  </ItemWrapper>
                );
              })}
            </ol>
          )}
        </div>
      </aside>
    </div>
  );
}

/** Render a transcript row, prefixed by an optional "Turn N" divider. */
function ItemWrapper({ divider, children }: { divider: ReactNode; children: ReactNode }) {
  return (
    <>
      {divider}
      <li>{children}</li>
    </>
  );
}
