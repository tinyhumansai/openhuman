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
import createDebug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { BubbleMarkdown } from '../../features/conversations/components/AgentMessageBubble';
import { ToolTimelineBlock } from '../../features/conversations/components/ToolTimelineBlock';
import { useWorkflowBuilderChat } from '../../hooks/useWorkflowBuilderChat';
import { unwrapToolCallEnvelope } from '../../lib/flows/copilotMessageSanitizer';
import { diffGraphs } from '../../lib/flows/graphDiff';
import type { WorkflowGraph } from '../../lib/flows/types';
import { useT } from '../../lib/i18n/I18nContext';
import type { WorkflowProposal } from '../../store/chatRuntimeSlice';
import ChatComposer from '../chat/ChatComposer';
import Button from '../ui/Button';
import ToolActivityChip from './ToolActivityChip';

const log = createDebug('app:flows:copilot-panel');

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
   * Fires once the build seed has been dispatched, so the host can clear the
   * ephemeral route seed (`location.state.copilotBuild`). The in-mount
   * `buildSentRef` guard only protects the current mount; closing and reopening
   * the panel remounts it and resets that ref, so without clearing the route
   * seed a remount would re-fire the same `build` turn (issue #4597).
   */
  onBuildSeedConsumed?: () => void;
  /**
   * Optional prefill seed (from the Suggested Workflows "Build this" action)
   * — populates the composer's input with the suggestion's `build_prompt`
   * once on mount WITHOUT sending it; the user reviews/edits the text and
   * presses Send themselves. Distinct from `buildSeed`, which auto-sends.
   *
   * `mode` carries the builder mode the FIRST Send after this prefill must
   * use — `'build'` for a Suggested Workflows seed, matching the
   * already-created blank flow's `BuildMode::Build` contract — instead of
   * `submit`'s normal `'revise'` turn. Consumed (and reset to `'revise'`) as
   * soon as that first Send fires; later Sends on the same mount are plain
   * revise turns.
   */
  prefillSeed?: { text: string; mode?: 'build' | 'create' } | null;
  /**
   * Fires once the prefill seed has populated the input, so the host can
   * clear the ephemeral route seed (`location.state.copilotPrefill`) — same
   * rationale as `onBuildSeedConsumed`: a remount (close/reopen) must not
   * re-populate the input a second time against a still-present route seed.
   */
  onPrefillSeedConsumed?: () => void;
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
  onBuildSeedConsumed,
  prefillSeed = null,
  onPrefillSeedConsumed,
  seedThreadId = null,
  onThreadIdChange,
}: Props) {
  const { t } = useT();
  const {
    threadId,
    sending,
    proposal,
    capped,
    displayMessages,
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

  // Holds the ORIGINAL ask when a turn ends without a proposal — i.e. the
  // agent asked a genuinely-ambiguous clarifying question (the prompt's
  // "bucket 3" branch) and stopped rather than revising. `submit` always
  // sends `mode: 'revise'` with the CURRENT graph, but while a question is
  // still open that graph hasn't changed yet, so a bare follow-up answer
  // ("#eng") would be the agent's ENTIRE context for the next turn — the
  // original request ("post a daily summary to Slack") would be lost and the
  // turn renders as "Revise it as follows: #eng" against a stale/blank draft.
  // Prepending the unresolved ask keeps that context alive across the Q&A
  // round-trip; it's cleared once a turn actually proposes (the graph itself
  // then carries the state, so later revises don't need it).
  const pendingAskRef = useRef<string | null>(null);

  // Sets/clears `pendingAskRef` after a turn settles, logging the decision
  // (stable prefix + thread correlation, never the raw ask/answer text — that
  // may carry user-authored content).
  const updatePendingAsk = useCallback(
    (proposed: boolean, ask: string) => {
      log(
        'pendingAsk: %s thread=%s',
        proposed ? 'cleared (proposal landed)' : 'set (still open)',
        threadId
      );
      pendingAskRef.current = proposed ? null : ask;
    },
    [threadId]
  );

  // Auto-send the repair turn once when opened from a failed run.
  const repairSentRef = useRef(false);
  useEffect(() => {
    if (!repairSeed || repairSentRef.current) return;
    repairSentRef.current = true;
    const instruction = t('flows.copilot.repairDisplay');
    send({
      displayText: instruction,
      request: {
        mode: 'repair',
        instruction: '',
        graph: repairSeed.graph,
        runId: repairSeed.runId,
        error: repairSeed.error ?? null,
        failingNodeIds: repairSeed.failingNodeIds ?? [],
      },
    }).then(({ proposed }) => {
      updatePendingAsk(proposed, instruction);
    });
  }, [repairSeed, send, t, updatePendingAsk]);

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
    // Optimistically guard re-entry while the async dispatch is in flight.
    buildSentRef.current = true;
    send({
      displayText: buildSeed.description,
      request: flowId
        ? { mode: 'build', instruction: buildSeed.description, graph, flowId }
        : { mode: 'revise', instruction: buildSeed.description, graph, flowId },
    }).then(({ outcome, proposed }) => {
      if (outcome === 'dispatched') {
        // Clear the ephemeral route seed only once the turn actually
        // dispatched, so closing and reopening the panel (which remounts it
        // and resets `buildSentRef`) can't re-fire the same build turn
        // (issue #4597).
        onBuildSeedConsumed?.();
      } else if (outcome === 'skipped') {
        // Retryable no-op (socket not connected yet, or a turn already in
        // flight): keep the seed and release the guard so the effect retries
        // once `send` changes identity on reconnect — otherwise the prompt is
        // lost and the blank flow never auto-builds.
        buildSentRef.current = false;
      }
      // `failed`: the dispatch was attempted but errored (surfaced via
      // `error`). Leave the guard set so THIS mount doesn't auto-resend and
      // duplicate the turn — the user retries from the input instead. Note the
      // route seed is deliberately NOT consumed on failure, so a later close +
      // reopen remounts the panel with a fresh `buildSentRef` and WILL re-fire
      // the build: a reopen is thus an intentional retry, not a manual-only one.
      // That's safe/desired — a failed turn persisted nothing (`mode:'build'`
      // never saves; issue #4596), so there's no partial state to clean up.
      //
      // Regardless of outcome, record whether the turn proposed: when it didn't
      // (a clarifying question, or a no-op/failed turn that never built), carry
      // the original description forward so the user's free-text answer (via
      // `submit` below) doesn't strand the agent with no idea what it was asked
      // to build. A later turn that proposes clears it.
      updatePendingAsk(proposed, buildSeed.description);
    });
    // `graph`/`flowId` are read once for the seed turn — later edits must not
    // re-fire it (guarded by the ref regardless).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [buildSeed, send, updatePendingAsk]);

  // Populate the composer's input once when opened with a Suggested Workflows
  // prefill seed — deliberately NEVER calls `send`: the user reviews/edits the
  // pre-filled `build_prompt` and presses Send themselves. Guarded the same
  // way as `buildSentRef`/`repairSentRef` (once per mount) so a re-render
  // doesn't re-fill (and clobber) text the user has already started editing.
  const prefillSentRef = useRef(false);
  // The builder mode the FIRST manual Send after this prefill must use (see
  // `prefillSeed`'s doc comment) — read and cleared by `submit` below once
  // that first Send actually fires, so later Sends fall back to `revise`.
  const pendingPrefillModeRef = useRef<'build' | 'create' | null>(null);
  useEffect(() => {
    if (!prefillSeed || prefillSentRef.current) return;
    prefillSentRef.current = true;
    pendingPrefillModeRef.current = prefillSeed.mode ?? 'build';
    log('prefill seed: populating composer input (unsent), pending mode=%s', prefillSeed.mode);
    setText(prefillSeed.text);
    textInputRef.current?.focus();
    // Consumed synchronously (no async dispatch to await, unlike build/repair)
    // so the host can strip the ephemeral route seed right away — a later
    // remount (close/reopen the copilot) then has no seed left to re-apply.
    onPrefillSeedConsumed?.();
  }, [prefillSeed, onPrefillSeedConsumed]);

  // Keep the transcript pinned to the newest message / streamed activity.
  // `scrollTo` is optional-chained: jsdom (tests) doesn't implement it.
  useEffect(() => {
    scrollRef.current?.scrollTo?.({ top: scrollRef.current.scrollHeight });
  }, [displayMessages, sending, proposal, toolTimeline, liveResponse]);

  const submit = useCallback(
    async (raw?: string) => {
      const trimmed = (raw ?? text).trim();
      if (!trimmed || sending) return;
      setText('');
      const priorAsk = pendingAskRef.current;
      const instruction = priorAsk
        ? `${priorAsk}\n\n(This is my answer to your question above: ${trimmed})`
        : trimmed;
      // The FIRST Send after a Suggested Workflows prefill seed must run the
      // seed's builder mode (default `build`), not the usual `revise` — the
      // seed's flow was just created blank, so this turn needs the `build`
      // brief (build → dry-run → propose) rather than being treated as a
      // revise of an existing draft. Consumed once: later Sends on this same
      // mount fall back to plain `revise`. Requires a real `flowId` (always
      // true for a prefill seed, which only ever seeds an existing flow's
      // canvas) — falls back to `revise` defensively if it's somehow absent,
      // mirroring `buildSeed`'s own fallback above.
      const prefillMode = pendingPrefillModeRef.current;
      pendingPrefillModeRef.current = null;
      const request =
        prefillMode && flowId
          ? { mode: prefillMode, instruction, graph, flowId }
          : { mode: 'revise' as const, instruction, graph, flowId };
      const { proposed } = await send({ displayText: trimmed, request });
      updatePendingAsk(proposed, instruction);
    },
    [text, sending, send, graph, flowId, updatePendingAsk]
  );

  // (B34) One-click resume for a turn that hit the agent's tool-call budget
  // (`capped`, see `useWorkflowBuilderChat`'s doc) with no proposal yet.
  // Routes through the SAME `submit` path a typed follow-up would — the
  // `pendingAskRef` mechanism (set above, since a capped turn also has
  // `proposed === false`) automatically carries the original ask forward, so
  // the agent picks the build back up with full context, not just "continue"
  // in isolation.
  //
  // What this actually does (Codex review on #4865): `flows_build` spins up a
  // FRESH `workflow_builder` agent per RPC — there is no server-side
  // session/tool-history checkpoint to reattach to, so this is not a literal
  // mid-thought resume. What DOES carry forward, because `submit` always
  // sends `mode: 'revise'` over the CURRENT `graph` + `flowId` (never a blank
  // `create`): (1) the live draft graph — unchanged by a capped turn, since
  // `revise_workflow`/`propose_workflow` never persist without a proposal
  // reaching this panel; and (2) the full accumulated instruction text via
  // `pendingAskRef`. A fresh agent re-reading the same draft plus the same
  // ask, now under the B31 50-iteration budget and B32's no-probing brief,
  // reliably converges — that combination is what the capped card's copy
  // promises ("keep building from the current draft"), not seamless
  // tool-history continuity.
  const continueBuilding = useCallback(() => {
    void submit(t('flows.copilot.continueBuilding'));
  }, [submit, t]);

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
  // B25: the in-flight streaming text can also carry the raw tool-call
  // envelope mid-turn — unwrap once and reuse the clean text everywhere below
  // (the pre-tool streaming bubble and the shared `ToolTimelineBlock`).
  const liveResponseText = unwrapToolCallEnvelope(liveResponse).text;
  const hasLiveText = liveResponseText.trim().length > 0;
  const isEmpty =
    displayMessages.length === 0 && !proposal && !sending && !error && !hasTimeline && !hasLiveText;

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

        {/* Conversation transcript: user turns right-aligned, agent turns left.
            Renders `displayMessages` (interim narration bubbles filtered out —
            that narration already streams via the tool timeline / live text
            below it double-renders it as a bubble too, see B4). */}
        {displayMessages.map(message => {
          if (message.sender === 'user') {
            return (
              <div
                key={message.id}
                className="flex justify-end"
                data-testid="workflow-copilot-user">
                <div className="max-w-[85%] rounded-2xl bg-primary-500 px-3 py-1.5 text-sm text-content-inverted">
                  {message.content}
                </div>
              </div>
            );
          }
          // B25: a turn that both talks and calls a tool can carry the
          // provider wire-format `{ content, tool_calls }` envelope as this
          // message's raw `content` — unwrap it so the bubble renders only
          // the human text (+ a compact tool-activity chip), never raw JSON.
          const { text, toolNames } = unwrapToolCallEnvelope(message.content);
          return (
            <div
              key={message.id}
              className="max-w-[92%] rounded-2xl bg-surface-subtle px-3 py-1.5"
              data-testid="workflow-copilot-agent">
              <BubbleMarkdown content={text} />
              <ToolActivityChip toolNames={toolNames} />
            </div>
          );
        })}

        {/* Live builder activity — the SHARED tool timeline (tool cards + the
            streaming reply) the main chat uses, fed from the runtime's streamed
            per-thread state. Renders nothing until the turn produces a tool
            call. */}
        {hasTimeline && (
          <div data-testid="workflow-copilot-timeline">
            <ToolTimelineBlock
              entries={toolTimeline}
              liveResponse={hasLiveText ? liveResponseText : undefined}
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
            <BubbleMarkdown content={liveResponseText} />
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

        {/* (B34) The turn hit the agent's iteration limit with no proposal
            yet — distinguish this from a voluntary clarifying question (which
            renders as a plain agent bubble above, no card) with an explicit
            "reached its iteration limit" signal and a one-click resume that
            continues building from the current draft (see `continueBuilding`
            above for why this is accurate rather than a seamless resume).
            Never shown alongside `sending` (a fresh turn already cleared
            `capped`) or a proposal (mutually exclusive server-side — see
            `ops.rs`). */}
        {capped && !sending && !proposal && (
          <div
            data-testid="workflow-copilot-capped"
            className="rounded-xl border border-amber-300 bg-surface p-3 dark:border-amber-700">
            <p className="text-xs text-content-secondary">{t('flows.copilot.cappedNotice')}</p>
            <div className="mt-2">
              <Button
                type="button"
                variant="secondary"
                size="sm"
                data-testid="workflow-copilot-continue"
                onClick={continueBuilding}>
                {t('flows.copilot.continueBuilding')}
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
