import { type ReactNode, useEffect, useState } from 'react';

import Button from '../../../components/ui/Button';
import { useT } from '../../../lib/i18n/I18nContext';
import { threadApi } from '../../../services/api/threadApi';
import type {
  SubagentActivity,
  SubagentTranscriptItem,
  ToolTimelineEntryStatus,
} from '../../../store/chatRuntimeSlice';
import type { ThreadMessage } from '../../../types/thread';
import { stripToolCallEnvelopes } from '../../../utils/toolTimelineFormatting';
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
  label:
    | 'statusRunning'
    | 'statusCompleted'
    | 'statusFailed'
    | 'statusAwaitingUser'
    | 'statusCancelled';
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
  if (status === 'cancelled') {
    return {
      dot: 'bg-stone-400 dark:bg-neutral-500',
      pill: 'bg-surface-subtle dark:bg-neutral-700/40 text-content-secondary',
      label: 'statusCancelled',
    };
  }
  if (status === 'awaiting_user') {
    return {
      dot: 'bg-amber-400 animate-pulse',
      pill: 'bg-amber-100 dark:bg-amber-500/20 text-amber-700 dark:text-amber-300',
      label: 'statusAwaitingUser',
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
  onCancel,
  onClose,
}: {
  subagent: SubagentActivity | null;
  /** Lifecycle status of the owning timeline row (running/success/error). */
  status?: ToolTimelineEntryStatus;
  /**
   * Cancel this still-running detached sub-agent. When provided and the run is
   * running, a "Cancel task" affordance is shown. The parent owns the actual
   * abort + chat delivery (via `subagentApi.cancel`); the drawer only manages
   * the in-flight / error UI and closes on success. Rejecting surfaces an error.
   */
  onCancel?: () => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useT();
  // Cancel-in-flight + last-error state for the "Cancel task" affordance.
  // The parent keys this drawer by task id, so a different sub-agent remounts
  // with fresh state — no effect-driven reset needed (which would trip the
  // repo's `react-hooks/set-state-in-effect` rule).
  const [cancelling, setCancelling] = useState(false);
  const [cancelError, setCancelError] = useState(false);

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
  const isRunning = status !== 'success' && status !== 'error' && status !== 'cancelled';
  // The "Cancel task" CTA is only meaningful for a live, still-running run the
  // parent gave us a cancel handler for.
  const canCancel = status === 'running' && Boolean(onCancel);

  const handleCancel = async () => {
    if (!onCancel || cancelling) return;
    setCancelling(true);
    setCancelError(false);
    try {
      await onCancel();
      // Success: the parent flips the row to cancelled and the notice rides the
      // idle-delivery path into chat — close the drawer.
      onClose();
    } catch {
      setCancelling(false);
      setCancelError(true);
    }
  };
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
      <aside className="relative flex h-full w-full max-w-md flex-col bg-surface shadow-xl">
        {/* Header */}
        <header className="flex items-center gap-2.5 border-b border-line px-4 py-3">
          <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary-50 dark:bg-primary-500/15 text-base">
            🤖
          </span>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="truncate font-semibold text-content">{subagent.agentId}</span>
              <span className={`h-2 w-2 shrink-0 rounded-full ${tone.dot}`} />
            </div>
            <div className="flex flex-wrap items-center gap-1.5 text-[11px] text-content-muted">
              <span className={`rounded-full px-1.5 py-0.5 ${tone.pill}`}>
                {t(`conversations.subagent.${tone.label}`)}
              </span>
              {subagent.childIteration != null ? (
                <span>
                  {subagent.childMaxIterations != null
                    ? `${t('conversations.toolTimeline.turn')} ${subagent.childIteration}/${subagent.childMaxIterations}`
                    : `${t('conversations.toolTimeline.step')} ${subagent.childIteration}`}
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
          {canCancel ? (
            <Button
              variant="secondary"
              tone="danger"
              size="sm"
              onClick={handleCancel}
              disabled={cancelling}
              data-testid="subagent-cancel"
              className="shrink-0 rounded-full">
              {cancelling
                ? t('conversations.subagent.cancelling')
                : t('conversations.subagent.cancel')}
            </Button>
          ) : null}
          <Button
            iconOnly
            variant="tertiary"
            size="sm"
            onClick={onClose}
            aria-label={t('conversations.subagent.close')}
            className="shrink-0 rounded-full">
            ✕
          </Button>
        </header>
        {cancelError ? (
          <div
            role="alert"
            data-testid="subagent-cancel-error"
            className="border-b border-coral-200 bg-coral-50 px-4 py-2 text-xs text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
            {t('conversations.subagent.cancelFailed')}
          </div>
        ) : null}

        {/* Body — a parent↔subagent conversation: the parent's delegation
            prompt opens it, then the sub-agent replies as one chronological
            transcript (thinking, the text it produced, the tool calls that
            text triggered, the next turn — exactly as it was emitted). */}
        <div className="flex-1 space-y-3 overflow-y-auto px-4 py-4">
          {/* Parent → sub-agent: the delegation prompt (the "input"). */}
          {promptText ? (
            <div className="flex justify-end" data-testid="subagent-parent-prompt">
              <div className="max-w-[85%] rounded-2xl rounded-br-md bg-primary-500 px-3 py-2 text-sm text-content-inverted">
                <div className="mb-0.5 text-[10px] font-semibold uppercase tracking-wide text-white/70">
                  {t('conversations.subagent.parent')}
                </div>
                <div className="whitespace-pre-wrap break-words">{promptText}</div>
              </div>
            </div>
          ) : null}

          {/* Sub-agent side: avatar label + its turns. */}
          <div className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-content-faint">
            <span>🤖</span>
            {subagent.agentId}
          </div>

          {transcript.length === 0 ? (
            <p className="text-xs italic text-content-faint">
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
                    className="flex items-center gap-2 pt-1 text-[10px] font-medium uppercase tracking-wide text-content-faint"
                    data-testid="subagent-turn-divider">
                    <span className="h-px flex-1 bg-surface-strong" />
                    {t('conversations.toolTimeline.turn')} {item.iteration}
                    <span className="h-px flex-1 bg-surface-strong" />
                  </li>
                ) : null;

                if (item.kind === 'thinking') {
                  return (
                    <ItemWrapper key={`th-${idx}`} divider={turnDivider}>
                      <div
                        className="rounded-lg bg-surface-muted px-3 py-2"
                        data-testid="subagent-transcript-thinking">
                        <div className="mb-1 flex items-center gap-1.5 text-[11px] font-semibold text-content-muted">
                          <span className="inline-block h-1.5 w-1.5 rounded-full bg-primary-400" />
                          {t('conversations.subagent.thinking')}
                        </div>
                        <pre className="whitespace-pre-wrap break-words font-sans text-[12px] leading-relaxed text-content-secondary">
                          {stripToolCallEnvelopes(item.text).trim()}
                        </pre>
                      </div>
                    </ItemWrapper>
                  );
                }

                if (item.kind === 'text') {
                  return (
                    <ItemWrapper key={`tx-${idx}`} divider={turnDivider}>
                      <div data-testid="subagent-transcript-text">
                        <BubbleMarkdown content={stripToolCallEnvelopes(item.text)} />
                        {isRunning && idx === lastTextIdx ? (
                          <span className="ml-0.5 inline-block h-3 w-1 animate-pulse bg-primary-400 align-middle" />
                        ) : null}
                      </div>
                    </ItemWrapper>
                  );
                }

                return (
                  <ItemWrapper key={`tl-${item.callId}`} divider={turnDivider}>
                    <ToolCallRow item={item} />
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

type SubagentToolItem = Extract<SubagentTranscriptItem, { kind: 'tool' }>;

/**
 * Pretty-print a tool's input arguments for display. Objects/arrays are
 * rendered as indented JSON; a string is shown verbatim. Returns `null` when
 * there are no arguments to show (e.g. a tool called with no input, or a
 * transcript reopened from memory where args weren't persisted).
 */
function formatArgs(args: unknown): string | null {
  if (args == null) return null;
  if (typeof args === 'string') return args.length > 0 ? args : null;
  try {
    return JSON.stringify(args, null, 2);
  } catch {
    return String(args);
  }
}

/**
 * One child tool call in the drawer transcript, expandable to reveal exactly
 * *what happened*: the input arguments the sub-agent passed and the raw output
 * the tool returned. Collapsed by default to keep the transcript scannable;
 * the chevron only appears once there's detail to reveal (args present, or the
 * call completed with a captured result). Reopened-from-memory transcripts
 * carry no args/result, so those rows stay non-expandable.
 */
function ToolCallRow({ item }: { item: SubagentToolItem }) {
  const { t } = useT();
  const [expanded, setExpanded] = useState(false);

  const callTone =
    item.status === 'running'
      ? 'text-amber-700 dark:text-amber-300'
      : item.status === 'success'
        ? 'text-sage-700 dark:text-sage-300'
        : item.status === 'cancelled'
          ? 'text-content-secondary'
          : item.status === 'awaiting_user'
            ? 'text-amber-700 dark:text-amber-300'
            : 'text-coral-700 dark:text-coral-300';
  const statusLabel =
    item.status === 'running'
      ? t('conversations.subagent.statusRunning')
      : item.status === 'success'
        ? t('conversations.subagent.statusCompleted')
        : item.status === 'cancelled'
          ? t('conversations.subagent.statusCancelled')
          : item.status === 'awaiting_user'
            ? t('conversations.subagent.statusAwaitingUser')
            : t('conversations.subagent.statusFailed');

  const argsText = formatArgs(item.args);
  const hasOutput = item.result != null;
  const expandable = argsText != null || hasOutput;

  const detailPre =
    'max-h-60 overflow-auto whitespace-pre-wrap break-words rounded bg-surface px-2 py-1.5 ' +
    'font-mono text-[11px] leading-relaxed text-content-secondary dark:bg-surface';
  const detailLabel = 'mb-1 text-[10px] font-semibold uppercase tracking-wide text-content-faint';

  return (
    <div
      className="rounded-md border border-line bg-surface-muted text-xs"
      data-testid="subagent-drawer-tool-call">
      <button
        type="button"
        disabled={!expandable}
        onClick={() => setExpanded(v => !v)}
        aria-expanded={expandable ? expanded : undefined}
        data-testid="subagent-tool-call-toggle"
        className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left disabled:cursor-default">
        {expandable ? (
          <span className="shrink-0 text-[9px] text-content-faint">{expanded ? '▾' : '▸'}</span>
        ) : (
          <span className="w-[9px] shrink-0" aria-hidden />
        )}
        <span className={callTone}>🔧</span>
        <span className="font-mono text-content-secondary">{item.toolName}</span>
        <span className={`ml-auto ${callTone}`}>{statusLabel}</span>
        {item.elapsedMs != null && item.status !== 'running' ? (
          <span className="text-[10px] text-content-faint">{formatElapsed(item.elapsedMs)}</span>
        ) : null}
      </button>
      {expandable && expanded ? (
        <div className="space-y-2 border-t border-line px-2.5 py-2">
          {argsText != null ? (
            <div data-testid="subagent-tool-call-input">
              <div className={detailLabel}>{t('conversations.subagent.input')}</div>
              <pre className={detailPre}>{argsText}</pre>
            </div>
          ) : null}
          {hasOutput ? (
            <div data-testid="subagent-tool-call-output">
              <div className={detailLabel}>{t('conversations.subagent.output')}</div>
              <pre className={detailPre}>
                {item.result && item.result.length > 0
                  ? item.result
                  : t('conversations.subagent.noOutput')}
              </pre>
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
