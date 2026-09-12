import createDebug from 'debug';
import { type ReactNode, useEffect, useState } from 'react';

import Badge, { type BadgeVariant } from '../../../components/ui/Badge';
import Button from '../../../components/ui/Button';
import { SheetContent, SheetRoot, SheetTitle } from '../../../components/ui/Sheet';
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
import { AssistantUiToolCallCard } from './AssistantUiToolCall';

const log = createDebug('app:conversations:subagent-drawer');

function formatElapsed(ms: number): string {
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`;
}

function subagentStatusVariant(status: ToolTimelineEntryStatus | undefined): BadgeVariant {
  if (status === 'success') return 'success';
  if (status === 'error') return 'danger';
  if (status === 'cancelled') return 'neutral';
  return 'warning';
}

function useSubagentStatusLabel(status: ToolTimelineEntryStatus | undefined): string {
  const { t } = useT();
  if (status === 'success') return t('conversations.subagent.statusCompleted');
  if (status === 'error') return t('conversations.subagent.statusFailed');
  if (status === 'cancelled') return t('conversations.subagent.statusCancelled');
  if (status === 'awaiting_user') return t('conversations.subagent.statusAwaitingUser');
  return t('conversations.subagent.statusRunning');
}

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
 * The status dot beside the sub-agent's name. Only the *dot* is hand-drawn —
 * the textual status is a shared {@link Badge}, so the tone vocabulary lives
 * in `subagentStatusVariant` and this maps the same statuses to the matching
 * fill.
 */
function statusDot(status: ToolTimelineEntryStatus | undefined): string {
  switch (status) {
    case 'success':
      return 'bg-sage-500';
    case 'error':
      return 'bg-coral-500';
    case 'cancelled':
      return 'bg-content-faint';
    case 'awaiting_user':
      return 'bg-amber-400 animate-pulse';
    default:
      return 'bg-amber-500 animate-pulse';
  }
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
 *
 * The overlay itself is the shared Radix-backed {@link SheetRoot}: the
 * hand-rolled `createPortal` + backdrop `<button>` + `keydown` listener it
 * replaced had no focus trap, no scroll lock and no focus restore on close.
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

  const statusLabel = useSubagentStatusLabel(status);

  useEffect(() => {
    if (!needsFetch || !workerThreadId) {
      setFetched(null);
      return;
    }
    // Clear any prior thread's transcript up front so it can't linger while
    // the new request is in flight.
    setFetched(null);
    let cancelled = false;
    log('reopen-from-memory: fetching worker thread %s', workerThreadId);
    void threadApi
      .getThreadMessages(workerThreadId)
      .then(data => {
        log('reopen-from-memory: %s returned %d messages', workerThreadId, data.messages.length);
        if (!cancelled) setFetched({ workerThreadId, ...transcriptFromMessages(data.messages) });
      })
      .catch(() => {
        log('reopen-from-memory: fetch failed for %s', workerThreadId);
        if (!cancelled) setFetched(null);
      });
    return () => {
      cancelled = true;
    };
  }, [needsFetch, workerThreadId]);

  if (!subagent) return null;

  const isRunning = status !== 'success' && status !== 'error' && status !== 'cancelled';
  // The "Cancel task" CTA is only meaningful for a live, still-running run the
  // parent gave us a cancel handler for.
  const canCancel = status === 'running' && Boolean(onCancel);

  const handleCancel = async () => {
    if (!onCancel || cancelling) return;
    log('cancel requested for task %s', subagent.taskId);
    setCancelling(true);
    setCancelError(false);
    try {
      await onCancel();
      // Success: the parent flips the row to cancelled and the notice rides the
      // idle-delivery path into chat — close the drawer.
      log('cancel succeeded for task %s', subagent.taskId);
      onClose();
    } catch {
      log('cancel FAILED for task %s', subagent.taskId);
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
    // `open` is hard-coded because this component renders nothing when there is
    // no subagent (the early return above) — `onOpenChange` is what routes
    // Escape / outside-click back to the caller's `onClose`.
    <SheetRoot
      open
      onOpenChange={next => {
        if (!next) onClose();
      }}>
      <SheetContent
        side="right"
        aria-describedby={undefined}
        data-testid="subagent-drawer"
        className="max-w-md">
        {/* Header */}
        <header className="flex shrink-0 items-center gap-2.5 border-b border-line px-4 py-3">
          <span
            aria-hidden
            className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary-50 text-base dark:bg-primary-500/15">
            🤖
          </span>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              {/* `asChild` keeps the historical inline span so the drawer's
                  layout is unchanged while Radix gets its required title. */}
              <SheetTitle asChild>
                <span className="truncate font-semibold text-content">{subagent.agentId}</span>
              </SheetTitle>
              <span aria-hidden className={`h-2 w-2 shrink-0 rounded-full ${statusDot(status)}`} />
            </div>
            <div className="flex flex-wrap items-center gap-1.5 text-[11px] text-content-muted">
              <Badge variant={subagentStatusVariant(status)}>{statusLabel}</Badge>
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
            className="shrink-0 border-b border-coral-200 bg-coral-50 px-4 py-2 text-xs text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
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
                <div className="mb-0.5 text-[10px] font-semibold uppercase tracking-wide text-content-inverted/70">
                  {t('conversations.subagent.parent')}
                </div>
                <div className="whitespace-pre-wrap wrap-break-word">{promptText}</div>
              </div>
            </div>
          ) : null}

          {/* Sub-agent side: avatar label + its turns. */}
          <div className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-content-faint">
            <span aria-hidden>🤖</span>
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
                          <span
                            aria-hidden
                            className="inline-block h-1.5 w-1.5 rounded-full bg-primary-400"
                          />
                          {t('conversations.subagent.thinking')}
                        </div>
                        <pre className="whitespace-pre-wrap wrap-break-word font-sans text-[12px] leading-relaxed text-content-secondary">
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
                          <span
                            aria-hidden
                            className="ml-0.5 inline-block h-3 w-1 animate-pulse bg-primary-400 align-middle"
                          />
                        ) : null}
                      </div>
                    </ItemWrapper>
                  );
                }

                return (
                  <ItemWrapper key={`tl-${item.callId}`} divider={turnDivider}>
                    <AssistantUiToolCallCard
                      toolName={item.toolName}
                      args={item.args}
                      result={item.result}
                      status={item.status}
                      displayName={item.displayName}
                      detail={item.detail}
                      elapsedMs={item.elapsedMs}
                      failure={item.failure}
                    />
                  </ItemWrapper>
                );
              })}
            </ol>
          )}
        </div>
      </SheetContent>
    </SheetRoot>
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
