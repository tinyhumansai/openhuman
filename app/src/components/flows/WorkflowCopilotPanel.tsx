/**
 * WorkflowCopilotPanel (Phase 5c) — a side-panel chat bound to the
 * `workflow_builder` specialist, docked on the editable canvas. The user asks
 * for changes ("add a Slack notification on failure", "make the schedule
 * weekdays only"); each turn injects the CURRENT draft graph as context and the
 * agent returns a `revise_workflow` proposal (and can now discover + connect the
 * Composio apps a step needs). The panel renders the full conversation
 * transcript, surfaces each proposal's node-level diff, and hands Accept/Reject
 * up to the host, which applies it to the local draft overlay.
 *
 * Chat UI parity: the copilot reuses the SHARED chat surface end-to-end — the
 * same {@link ChatComposer} the main chat windows use (mic/attachments off
 * here), turns render as bubbles via the shared {@link BubbleMarkdown}, and the
 * builder turn's live tool activity + streaming reply render through the shared
 * {@link ToolTimelineBlock} (fed from the runtime's `toolTimelineByThread` /
 * `streamingAssistantByThread`, streamed here by Phase B). So the copilot reads
 * like a real chat rather than a one-shot form.
 *
 * Invariant: the copilot only PROPOSES. Accept applies to the UNSAVED local
 * draft (no `flows_update`); persistence stays behind the canvas's own Save.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import { useWorkflowBuilderChat } from '../../hooks/useWorkflowBuilderChat';
import { diffGraphs } from '../../lib/flows/graphDiff';
import type { WorkflowGraph } from '../../lib/flows/types';
import { useT } from '../../lib/i18n/I18nContext';
import { BubbleMarkdown } from '../../pages/conversations/components/AgentMessageBubble';
import { ToolTimelineBlock } from '../../pages/conversations/components/ToolTimelineBlock';
import type { WorkflowProposal } from '../../store/chatRuntimeSlice';
import ChatComposer from '../chat/ChatComposer';
import Button from '../ui/Button';

/**
 * Context for a repair turn opened from a failed run's inspector ("Fix with
 * agent"). Maps directly onto a `repair`-mode builder request.
 */
export interface RepairPromptContext {
  /** The failed run id (== thread_id) so the agent can `get_flow_run` it. */
  runId: string;
  /** The run-level error message, if any. */
  error?: string | null;
  /** Node ids that failed / are implicated, if known. */
  failingNodeIds?: string[];
  /** The flow's current graph, injected so the fix builds on the real draft. */
  graph: WorkflowGraph;
}

interface Props {
  /** The current draft graph, injected as context for each revise turn. */
  graph: WorkflowGraph;
  /**
   * The saved flow's id (or `null`/absent for an unsaved draft), injected into
   * revise turns so the agent can `run_workflow` it to test — with confirmation.
   */
  flowId?: string | null;
  /**
   * Fires when the agent returns a fresh proposal, so the host can enter its
   * diff-preview overlay. The host computes/holds the preview; this panel only
   * reflects it.
   */
  onProposal: (proposal: WorkflowProposal) => void;
  /** Accept the pending proposal into the local draft (host commits it). */
  onAccept: (proposal: WorkflowProposal) => void;
  /** Reject the pending proposal (host reverts the overlay). */
  onReject: () => void;
  /** Close the panel. */
  onClose: () => void;
  /**
   * Optional repair seed (from a failed run's "Fix with agent") — auto-sends a
   * repair turn once on mount so the copilot opens already diagnosing.
   */
  repairSeed?: RepairPromptContext | null;
  /**
   * Optional build seed (from the Flows prompt bar's instant-create path) —
   * auto-sends the user's workflow description once on mount so the copilot
   * opens already building it against the just-created blank flow.
   */
  buildSeed?: { description: string } | null;
  /**
   * The workflow's persisted copilot thread id (from the per-flow cache), so
   * reopening the panel resumes the same conversation instead of starting fresh.
   */
  seedThreadId?: string | null;
  /** Reports the live thread id up so the host can persist it per workflow. */
  onThreadIdChange?: (threadId: string | null) => void;
}

export default function WorkflowCopilotPanel({
  graph,
  flowId = null,
  onProposal,
  onAccept,
  onReject,
  onClose,
  repairSeed = null,
  buildSeed = null,
  seedThreadId = null,
  onThreadIdChange,
}: Props) {
  const { t } = useT();
  const {
    threadId,
    sending,
    proposal,
    messages,
    toolTimeline,
    liveResponse,
    error,
    send,
    clearProposal,
  } = useWorkflowBuilderChat(seedThreadId);
  const [text, setText] = useState('');

  // Report the (lazily-created) thread id up so the host persists it per flow —
  // reopening the copilot then resumes this same conversation.
  useEffect(() => {
    onThreadIdChange?.(threadId);
  }, [threadId, onThreadIdChange]);

  // ChatComposer plumbing (mic/attachments are off, so most refs are inert).
  const textInputRef = useRef<HTMLTextAreaElement | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const isComposingTextRef = useRef(false);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  // Surface each NEW proposal to the host exactly once (enter preview overlay).
  const lastSurfacedRef = useRef<WorkflowProposal | null>(null);
  useEffect(() => {
    if (proposal && proposal !== lastSurfacedRef.current) {
      lastSurfacedRef.current = proposal;
      onProposal(proposal);
    }
  }, [proposal, onProposal]);

  // Auto-send the repair turn once when opened from a failed run.
  const repairSentRef = useRef(false);
  useEffect(() => {
    if (!repairSeed || repairSentRef.current) return;
    repairSentRef.current = true;
    void send({
      displayText: t('flows.copilot.repairDisplay'),
      request: {
        mode: 'repair',
        instruction: '',
        graph: repairSeed.graph,
        runId: repairSeed.runId,
        error: repairSeed.error ?? null,
        failingNodeIds: repairSeed.failingNodeIds ?? [],
      },
    });
  }, [repairSeed, send, t]);

  // Auto-send the build turn once when opened from the prompt bar's
  // instant-create path: the user's description becomes the first user turn on
  // this thread, and the prompt asks for the full build → dry-run → PROPOSE
  // arc against the just-created flow. Persistence still stays behind the
  // usual Accept + canvas Save; `mode: 'build'` intentionally does NOT save
  // the graph (issue #4596 — a Reject used to leave the graph persisted).
  // Falls back to a plain revise turn if the flow id is somehow missing.
  const buildSentRef = useRef(false);
  useEffect(() => {
    if (!buildSeed || buildSentRef.current) return;
    buildSentRef.current = true;
    void send({
      displayText: buildSeed.description,
      request: flowId
        ? { mode: 'build', instruction: buildSeed.description, graph, flowId }
        : { mode: 'revise', instruction: buildSeed.description, graph, flowId },
    });
    // `graph`/`flowId` are read once for the seed turn — later edits must not
    // re-fire it (guarded by the ref regardless).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [buildSeed, send]);

  // Keep the transcript pinned to the newest message / streamed activity.
  // `scrollTo` is optional-chained: jsdom (tests) doesn't implement it.
  useEffect(() => {
    scrollRef.current?.scrollTo?.({ top: scrollRef.current.scrollHeight });
  }, [messages, sending, proposal, toolTimeline, liveResponse]);

  const submit = useCallback(
    async (raw?: string) => {
      const trimmed = (raw ?? text).trim();
      if (!trimmed || sending) return;
      setText('');
      await send({
        displayText: trimmed,
        request: { mode: 'revise', instruction: trimmed, graph, flowId },
      });
    },
    [text, sending, send, graph, flowId]
  );

  const handleInputKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (event.key === 'Enter' && !event.shiftKey && !isComposingTextRef.current) {
        event.preventDefault();
        void submit();
      }
    },
    [submit]
  );

  const noopAttach = useCallback(async () => {}, []);
  const noop = useCallback(() => {}, []);

  const accept = useCallback(() => {
    if (!proposal) return;
    onAccept(proposal);
    clearProposal();
    lastSurfacedRef.current = null;
  }, [proposal, onAccept, clearProposal]);

  const reject = useCallback(() => {
    onReject();
    clearProposal();
    lastSurfacedRef.current = null;
  }, [onReject, clearProposal]);

  const diff = proposal ? diffGraphs(graph, proposal.graph as WorkflowGraph) : null;
  const hasTimeline = toolTimeline.length > 0;
  const hasLiveText = liveResponse.trim().length > 0;
  const isEmpty =
    messages.length === 0 && !proposal && !sending && !error && !hasTimeline && !hasLiveText;

  return (
    <aside
      data-testid="workflow-copilot-panel"
      className="flex h-full w-full max-w-sm flex-col border-l border-line bg-surface">
      <header className="flex items-start gap-2 border-b border-line px-3 py-2.5">
        <div className="min-w-0 flex-1">
          <p className="text-sm font-semibold text-content">{t('flows.copilot.title')}</p>
          <p className="text-[11px] text-content-muted">{t('flows.copilot.subtitle')}</p>
        </div>
        <button
          type="button"
          data-testid="workflow-copilot-close"
          aria-label={t('flows.copilot.close')}
          onClick={onClose}
          className="shrink-0 rounded-full p-1.5 text-content-faint hover:bg-surface-hover hover:text-content-secondary">
          ✕
        </button>
      </header>

      <div
        ref={scrollRef}
        className="flex-1 space-y-3 overflow-y-auto px-3 py-3"
        data-testid="workflow-copilot-transcript">
        {isEmpty && (
          <p className="text-xs text-content-muted" data-testid="workflow-copilot-empty">
            {t('flows.copilot.emptyState')}
          </p>
        )}

        {/* Conversation transcript: user turns right-aligned, agent turns left. */}
        {messages.map(message =>
          message.sender === 'user' ? (
            <div key={message.id} className="flex justify-end" data-testid="workflow-copilot-user">
              <div className="max-w-[85%] rounded-2xl bg-primary-500 px-3 py-1.5 text-sm text-content-inverted">
                {message.content}
              </div>
            </div>
          ) : (
            <div
              key={message.id}
              className="max-w-[92%] rounded-2xl bg-surface-subtle px-3 py-1.5"
              data-testid="workflow-copilot-agent">
              <BubbleMarkdown content={message.content} />
            </div>
          )
        )}

        {/* Live builder activity — the SHARED tool timeline (tool cards + the
            streaming reply) the main chat uses, fed from the runtime's streamed
            per-thread state. Renders nothing until the turn produces a tool
            call. */}
        {hasTimeline && (
          <div data-testid="workflow-copilot-timeline">
            <ToolTimelineBlock
              entries={toolTimeline}
              liveResponse={hasLiveText ? liveResponse : undefined}
            />
          </div>
        )}

        {/* Pre-tool phase: the reply is streaming but no tool has run yet, so the
            timeline is still empty — surface the live text as an agent bubble so
            the copilot never looks frozen. */}
        {hasLiveText && !hasTimeline && (
          <div
            className="max-w-[92%] rounded-2xl bg-surface-subtle px-3 py-1.5"
            data-testid="workflow-copilot-streaming">
            <BubbleMarkdown content={liveResponse} />
          </div>
        )}

        {sending && !hasTimeline && !hasLiveText && (
          <p className="text-xs text-content-muted" data-testid="workflow-copilot-thinking">
            {t('flows.copilot.thinking')}
          </p>
        )}

        {error && (
          <p className="text-xs text-coral" data-testid="workflow-copilot-error">
            {error === 'offline' ? t('flows.copilot.offline') : t('flows.copilot.error')}
          </p>
        )}

        {proposal && diff && (
          <div
            data-testid="workflow-copilot-proposal"
            className="rounded-xl border border-ocean-300 bg-surface p-3 dark:border-ocean-700">
            <p className="text-xs font-semibold text-ocean-900 dark:text-ocean-100">
              {proposal.name || t('flows.copilot.proposalTitle')}
            </p>
            <p className="mt-1 text-[11px] text-content-muted">{t('flows.copilot.previewHint')}</p>

            <div className="mt-2 flex flex-wrap gap-1.5 text-[11px]">
              {diff.addedNodeIds.size > 0 && (
                <span
                  data-testid="workflow-copilot-added"
                  className="rounded-full bg-sage-100 px-2 py-0.5 font-medium text-sage-700 dark:bg-sage-500/15 dark:text-sage-300">
                  {t('flows.copilot.added').replace('{count}', String(diff.addedNodeIds.size))}
                </span>
              )}
              {diff.removedNodeIds.size > 0 && (
                <span
                  data-testid="workflow-copilot-removed"
                  className="rounded-full bg-coral-100 px-2 py-0.5 font-medium text-coral-700 dark:bg-coral-500/15 dark:text-coral-300">
                  {t('flows.copilot.removed').replace('{count}', String(diff.removedNodeIds.size))}
                </span>
              )}
              {!diff.hasChanges && (
                <span className="text-content-faint">{t('flows.copilot.noChanges')}</span>
              )}
            </div>

            <div className="mt-3 flex items-center gap-2">
              <Button
                type="button"
                variant="primary"
                size="sm"
                data-testid="workflow-copilot-accept"
                onClick={accept}>
                {t('flows.copilot.accept')}
              </Button>
              <Button
                type="button"
                variant="secondary"
                size="sm"
                data-testid="workflow-copilot-reject"
                onClick={reject}>
                {t('flows.copilot.reject')}
              </Button>
            </div>
          </div>
        )}
      </div>

      <div className="border-t border-line px-3 py-2.5">
        <ChatComposer
          inputValue={text}
          setInputValue={setText}
          onSend={submit}
          textInputRef={textInputRef}
          fileInputRef={fileInputRef}
          composerInteractionBlocked={sending}
          isSending={sending}
          attachments={[]}
          onAttachFiles={noopAttach}
          onRemoveAttachment={noop}
          attachError={null}
          onSwitchToMicCloud={noop}
          handleInputKeyDown={handleInputKeyDown}
          inlineCompletionSuffix=""
          isComposingTextRef={isComposingTextRef}
          maxAttachments={0}
          allowedMimeTypes={[]}
          attachmentsEnabled={false}
          micEnabled={false}
          placeholder={t('flows.copilot.placeholder')}
        />
      </div>
    </aside>
  );
}
