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
 * Chat UI parity: the copilot renders its transcript through the SAME
 * {@link ChatThreadView} the home composer chat uses — message bubbles,
 * past-turn insights, the shared tool timeline + sub-agent drawer, and the
 * streaming / interrupted / parallel previews — driven by this copilot's
 * DEDICATED thread. `flows_build` streams the `workflow_builder` turn onto
 * that thread via the global `ChatRuntimeProvider` (Phase B), exactly as a
 * normal chat turn streams, so the copilot reads like the real chat rather
 * than a bespoke transcript. This panel keeps only the authoring concerns:
 * the {@link ChatComposer} footer (mic/attachments off), the seed auto-sends,
 * and the proposal-preview + capped cards pinned above the composer.
 *
 * Invariant: the copilot only PROPOSES — the agent turn itself never
 * persists. Accept applies the proposal to the local draft AND immediately
 * saves it (review + save in one click) via the host's `onAccept`, which
 * awaits the host's own persistence call; the panel shows a saving state
 * meanwhile and, if the save fails, leaves the proposal visible for retry
 * rather than silently discarding it. Reject remains local-only (revert the
 * overlay, no persistence call).
 */
import createDebug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { ChatThreadView } from '../../features/conversations/components/ChatThreadView';
import { useWorkflowBuilderChat } from '../../hooks/useWorkflowBuilderChat';
import { diffGraphs } from '../../lib/flows/graphDiff';
import type { WorkflowGraph } from '../../lib/flows/types';
import { useT } from '../../lib/i18n/I18nContext';
import type { WorkflowProposal } from '../../store/chatRuntimeSlice';
import ApprovalRequestCard from '../chat/ApprovalRequestCard';
import ChatComposer from '../chat/ChatComposer';
import IntegrationConnectCard from '../chat/IntegrationConnectCard';
import Button from '../ui/Button';

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
  /**
   * Accept the pending proposal (host applies it to the local draft AND
   * persists it — "accept" is now review + save in one step). May return a
   * promise the panel awaits to show a saving state; a rejected promise
   * leaves the proposal visible so the user can retry.
   *
   * `opts.enable` (PR1 — "Save & enable") requests an immediate follow-up
   * arm after the save succeeds, mirroring the main-chat
   * `WorkflowProposalCard`'s one-click create+arm. Optional and backward
   * compatible — a plain "Accept & save" click omits `opts` entirely, so it
   * neither enables nor force-disables an already-enabled existing flow.
   */
  onAccept: (proposal: WorkflowProposal, opts?: { enable?: boolean }) => void | Promise<void>;
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
  /**
   * Drop the panel's `max-w-sm` cap so it fills the available width. Used by
   * the chat-first canvas open, where the copilot is the whole surface until
   * the graph appears.
   */
  fullWidth?: boolean;
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
  fullWidth = false,
}: Props) {
  const { t } = useT();
  const { threadId, sending, proposal, pendingApproval, capped, error, send, clearProposal } =
    useWorkflowBuilderChat(seedThreadId);
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

  // Set only when a "Save & enable" attempt's `onAccept` rejects — surfaced
  // as a dedicated inline message (`flows.copilot.enableError`) distinct from
  // a plain "Accept & save" failure, which stays silent-but-retryable as
  // before (the button re-enabling is signal enough there). Declared early
  // (ahead of the proposal-surfacing effect below, which also clears it) so
  // both that effect and the accept/reject handlers further down can
  // reference it without a temporal-dead-zone ordering issue.
  const [enableError, setEnableError] = useState(false);

  // Surface each NEW proposal to the host exactly once (enter preview overlay).
  const lastSurfacedRef = useRef<WorkflowProposal | null>(null);
  useEffect(() => {
    if (proposal && proposal !== lastSurfacedRef.current) {
      lastSurfacedRef.current = proposal;
      // A genuinely new proposal object replacing a prior one (e.g. a further
      // revise turn) supersedes any stale "Save & enable" failure from the
      // earlier proposal — clear it so the new card doesn't inherit an
      // unrelated error message.
      setEnableError(false);
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

  // Transcript rendering + scroll pinning (stick-to-bottom) are owned by the
  // shared `ChatThreadView` below — the copilot no longer hand-rolls the
  // transcript. This component keeps only the authoring concerns: the
  // structured `flows_build` send path, the seed auto-sends, and the
  // proposal / capped cards surfaced in the footer.
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

  // Accept now review-and-saves: `onAccept` (the host's `handleAcceptProposal`)
  // applies the proposal to the draft AND persists it. Track a local
  // `acceptState` union (rather than a plain boolean) so the two accept
  // buttons ("Accept & save" / "Save & enable", PR1) can each show their own
  // in-flight label while BOTH stay disabled — a save-in-flight click on the
  // other button, or Reject, must not race the pending persist. If the
  // host's save (or enable) throws, leave the proposal card visible (don't
  // `clearProposal()`) so the user can retry — otherwise a failed autosave
  // would silently vanish the only affordance to try again from the copilot
  // itself (the header Save button is a fallback, but this keeps the
  // copilot's own flow self-contained).
  const [acceptState, setAcceptState] = useState<'idle' | 'saving' | 'enabling'>('idle');
  const acceptBusy = acceptState !== 'idle';
  const runAccept = useCallback(
    async (opts?: { enable?: boolean }) => {
      // Self-guard against re-entrance: the JSX `disabled={acceptBusy}` on
      // both buttons prevents a normal double-click, but `acceptState` only
      // flips after the FIRST call's `setAcceptState(...)` commits — a
      // second invocation racing ahead of that render (e.g. programmatic
      // re-fire) must not start a second concurrent save.
      if (!proposal || acceptBusy) return;
      const enable = Boolean(opts?.enable);
      setAcceptState(enable ? 'enabling' : 'saving');
      setEnableError(false);
      log('accept: saving proposal via host onAccept enable=%s', enable);
      try {
        // Plain "Accept & save" calls `onAccept` with just the proposal (no
        // second argument at all) — matching the pre-PR1 call signature
        // exactly — so a host that doesn't care about `opts` (or a caller
        // asserting on `onAccept`'s exact arguments) sees no behavioral
        // change. Only "Save & enable" adds the `{ enable: true }` opts.
        if (enable) {
          await onAccept(proposal, opts);
        } else {
          await onAccept(proposal);
        }
        log('accept: save succeeded, clearing proposal');
        clearProposal();
        lastSurfacedRef.current = null;
      } catch (err) {
        log('accept: save (or enable) failed, leaving proposal visible for retry err=%o', err);
        if (enable) setEnableError(true);
      } finally {
        setAcceptState('idle');
      }
    },
    [proposal, acceptBusy, onAccept, clearProposal, setEnableError]
  );
  const accept = useCallback(() => runAccept(), [runAccept]);
  const acceptAndEnable = useCallback(() => runAccept({ enable: true }), [runAccept]);

  const reject = useCallback(() => {
    onReject();
    clearProposal();
    lastSurfacedRef.current = null;
    setEnableError(false);
  }, [onReject, clearProposal, setEnableError]);

  const diff = proposal ? diffGraphs(graph, proposal.graph as WorkflowGraph) : null;

  return (
    <aside
      data-testid="workflow-copilot-panel"
      className={`flex h-full w-full flex-col border-l border-line bg-surface ${
        fullWidth ? '' : 'max-w-sm'
      }`}>
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

      {/* Full builder transcript — the SAME rich renderer the home composer
          chat uses (message bubbles, past-turn insights, the shared tool
          timeline + sub-agent drawer, streaming/interrupted/parallel previews),
          driven by this copilot's DEDICATED thread. `flows_build` streams the
          `workflow_builder` turn onto `threadId` via the global
          `ChatRuntimeProvider`, exactly as a normal chat turn streams, so the
          copilot now reads like the real chat instead of a bespoke transcript.
          The empty hint, proposal preview, and capped card are the copilot's
          own authoring affordances, kept in the footer below. */}
      <ChatThreadView
        threadId={threadId}
        variant="sidebar"
        scrollResetKey="workflow-copilot"
        shareAgentName={t('flows.copilot.title')}
        emptyContent={
          <div className="flex h-full items-center justify-center px-3">
            <p className="text-xs text-content-muted" data-testid="workflow-copilot-empty">
              {t('flows.copilot.emptyState')}
            </p>
          </div>
        }
      />

      <div className="space-y-3 border-t border-line px-3 py-2.5">
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

            <div className="mt-3 flex flex-wrap items-center gap-2">
              <Button
                type="button"
                variant="primary"
                size="sm"
                analyticsId="workflow-copilot-accept"
                disabled={acceptBusy}
                data-testid="workflow-copilot-accept"
                onClick={() => void accept()}>
                {acceptState === 'saving'
                  ? t('flows.copilot.saving')
                  : t('flows.copilot.acceptAndSave')}
              </Button>
              <Button
                type="button"
                variant="primary"
                size="sm"
                analyticsId="workflow-copilot-accept-and-enable"
                disabled={acceptBusy}
                data-testid="workflow-copilot-accept-and-enable"
                onClick={() => void acceptAndEnable()}>
                {acceptState === 'enabling'
                  ? t('flows.copilot.enabling')
                  : t('flows.copilot.saveAndEnable')}
              </Button>
              <Button
                type="button"
                variant="secondary"
                size="sm"
                disabled={acceptBusy}
                data-testid="workflow-copilot-reject"
                onClick={reject}>
                {t('flows.copilot.reject')}
              </Button>
            </div>
            {acceptState === 'idle' && enableError && (
              <p className="mt-2 text-xs text-coral" data-testid="workflow-copilot-enable-error">
                {t('flows.copilot.enableError')}
              </p>
            )}
          </div>
        )}

        {/* (B34) The turn hit the agent's iteration limit with no proposal
            yet — distinguish this from a voluntary clarifying question (which
            renders as a plain agent bubble in the transcript, no card) with an
            explicit "reached its iteration limit" signal and a one-click resume
            that continues building from the current draft (see `continueBuilding`
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

        {/* Parked ApprovalGate request for the copilot's dedicated thread (PR3:
            flows-copilot-live-run-approval). `flows_build` now runs under
            `AgentTurnOrigin::WebChat` + `APPROVAL_CHAT_CONTEXT` when streaming,
            so a `run_flow` / `resume_flow_run` call parks here instead of
            auto-allowing or being hidden — surfaced via the SAME
            `pendingApprovalByThread` slice / `approval_request` socket event
            `Conversations.tsx` reads for the main chat, reusing the identical
            cards (no new component, no new i18n keys). `composio_connect` parks
            on the same gate but needs a Connect button + OAuth poll rather than
            approve/deny, mirroring `Conversations.tsx`'s branch. Rendered above
            the composer, outside the scrollable transcript, so it stays visible
            regardless of scroll position. */}
        {pendingApproval && threadId && (
          <div data-testid="workflow-copilot-approval">
            {pendingApproval.toolName === 'composio_connect' ? (
              <IntegrationConnectCard
                key={pendingApproval.requestId}
                threadId={threadId}
                approval={pendingApproval}
              />
            ) : (
              <ApprovalRequestCard
                key={pendingApproval.requestId}
                threadId={threadId}
                approval={pendingApproval}
              />
            )}
          </div>
        )}

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
