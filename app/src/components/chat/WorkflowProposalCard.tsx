import debug from 'debug';
import React, { useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { FLOW_CANVAS_DRAFT_ROUTE, type FlowCanvasDraftState } from '../../lib/flows/canvasDraft';
import type { WorkflowGraph } from '../../lib/flows/types';
import { useT } from '../../lib/i18n/I18nContext';
import { createFlow, setFlowEnabled } from '../../services/api/flowsApi';
import {
  clearWorkflowProposalForThread,
  type WorkflowProposal,
} from '../../store/chatRuntimeSlice';
import { useAppDispatch } from '../../store/hooks';
import Button from '../ui/Button';

const log = debug('openhuman:chat:workflow-proposal-card');

interface Props {
  threadId: string;
  proposal: WorkflowProposal;
  /**
   * Optional callback fired after a successful "Save & enable" (the flow was
   * persisted via `flows_create`). The Flows page "Suggested for you" section
   * uses this to mark the originating suggestion as built so it drops out of
   * the active cards. Unused by the default chat/prompt-bar placements.
   */
  onSaved?: () => void;
}

/**
 * Human-in-the-loop gate for the `propose_workflow` agent tool (issue B4 —
 * agent-first Workflow authoring). The tool only VALIDATES a candidate
 * `tinyflows` graph and returns a summary — it can NEVER create or enable a
 * flow itself. This card is the only path from a proposal to a saved
 * automation: "Save & enable" calls `openhuman.flows_create` directly from
 * the client; the agent has no way to reach that RPC on its own. "Dismiss"
 * just clears the proposal without saving anything.
 *
 * B29 (save/enable safety) Rule 1 forces `flows_create` to persist an
 * automatic-trigger graph (`schedule` / `app_event` / `webhook`) DISABLED,
 * no matter what the caller passed — that's what stops a copilot
 * `save_workflow` autosave from silently arming an unattended automation.
 * But "Save & enable" is the user's own explicit arming click, not a silent
 * autosave, so when `createFlow` hands back a disabled flow here, `save`
 * follows up with an explicit {@link setFlowEnabled} call — the same toggle
 * `flows_set_enabled` exposes everywhere else — so the button does what it
 * says. If that follow-up enable call itself fails, the flow stays saved
 * (disabled) and the card keeps `savedFlowId` around so a retry re-enables
 * instead of re-creating (which would duplicate the flow).
 *
 * Mirrors {@link PlanReviewCard}'s placement/chrome above the composer, and
 * the tool-timeline `StatusTag`/detail-chip visual language for the
 * node-kind badges + config hints in the step list.
 */
export const WorkflowProposalCard: React.FC<Props> = ({ threadId, proposal, onSaved }) => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const [saving, setSaving] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  // Set once `createFlow` has persisted the flow but it came back disabled
  // (B29 Rule 1) and the follow-up `setFlowEnabled` call hasn't succeeded
  // yet. Non-null means a retry of `save` should skip `createFlow` entirely
  // and only retry the enable step.
  const [savedFlowId, setSavedFlowId] = useState<string | null>(null);

  const dismiss = () => {
    dispatch(clearWorkflowProposalForThread({ threadId }));
  };

  /**
   * Open the proposed graph in the editable Workflow Canvas as an UNSAVED
   * draft. This deliberately does NOT persist or enable anything — no
   * `flows_create`/`flows_update` — so the user can review/edit first; the
   * canvas's own Save button stays the single persistence gate. The proposal
   * is left intact in the thread (not dismissed) so returning without saving
   * loses nothing.
   */
  const openInCanvas = () => {
    const graph = proposal.graph as WorkflowGraph;
    // Log shape, not the user-authored `proposal.name` (no secrets/PII in logs).
    log(
      'openInCanvas: threadId=%s node_count=%d edge_count=%d',
      threadId,
      graph.nodes.length,
      graph.edges.length
    );
    const draft: FlowCanvasDraftState = {
      name: proposal.name,
      graph,
      requireApproval: proposal.requireApproval,
    };
    navigate(FLOW_CANVAS_DRAFT_ROUTE, { state: draft });
  };

  const save = async () => {
    if (saving) return;
    setSaving(true);
    setErrorMsg(null);
    // Track persistence locally (not via `savedFlowId` state) because a
    // `setState` call doesn't apply synchronously — reading `savedFlowId`
    // itself in the `catch` below would see this render's stale value, not
    // what just happened in this attempt.
    let flowId = savedFlowId;
    let flowPersisted = flowId !== null;
    try {
      if (!flowId) {
        const flow = await createFlow(proposal.name, proposal.graph, proposal.requireApproval);
        flowId = flow.id;
        flowPersisted = true;
        if (flow.enabled) {
          log('save: createFlow returned enabled — nothing further to arm id=%s', flow.id);
          dispatch(clearWorkflowProposalForThread({ threadId }));
          onSaved?.();
          return;
        }
        // B29 Rule 1 saved this automatic-trigger flow disabled. This click
        // is the user's own explicit "Save & enable" — not the copilot's
        // silent autosave Rule 1 guards against — so arm it now.
        log('save: createFlow returned disabled (Rule 1) — arming explicitly id=%s', flow.id);
        setSavedFlowId(flow.id);
      }
      await setFlowEnabled(flowId, true);
      dispatch(clearWorkflowProposalForThread({ threadId }));
      onSaved?.();
    } catch (e) {
      log('save failed (createFlow/setFlowEnabled): %o', e);
      setErrorMsg(
        flowPersisted ? t('chat.flowProposal.enableError') : t('chat.flowProposal.error')
      );
      setSaving(false);
    }
  };

  return (
    <div
      role="group"
      aria-label={t('chat.flowProposal.title')}
      data-testid="workflow-proposal-card"
      className="mb-2 rounded-xl border border-ocean-300 bg-surface p-3 text-sm shadow-md dark:border-ocean-700">
      <div className="flex items-start gap-2">
        <span aria-hidden className="text-base leading-none text-ocean-700 dark:text-ocean-200">
          ⚙️
        </span>
        <div className="min-w-0 flex-1">
          <p className="font-semibold text-ocean-900 dark:text-ocean-100">
            {proposal.name || t('chat.flowProposal.title')}
          </p>
          <p className="mt-1 break-words text-ocean-800/90 dark:text-ocean-200/90">
            {t('chat.flowProposal.subtitle')}
          </p>

          <p className="mt-2 text-xs break-words text-content-secondary">
            <span className="font-medium text-content-muted">
              {t('chat.flowProposal.triggerLabel')}:
            </span>{' '}
            {proposal.summary.trigger}
          </p>

          <div className="mt-2">
            <p className="text-xs font-medium text-content-muted">
              {t('chat.flowProposal.stepsLabel')}
            </p>
            {proposal.summary.steps.length > 0 ? (
              <ol className="mt-1 max-h-56 list-decimal overflow-y-auto pl-6 text-content-secondary">
                {proposal.summary.steps.map((step, i) => (
                  <li key={i} className="break-words">
                    <span
                      data-testid="workflow-proposal-step-kind"
                      className="mr-1.5 inline-block rounded-full bg-ocean-100 px-1.5 py-0.5 text-[10px] font-medium text-ocean-700 dark:bg-ocean-500/15 dark:text-ocean-300">
                      {step.kind}
                    </span>
                    <span>{step.name}</span>
                    {step.config_hint ? (
                      <span
                        title={step.config_hint}
                        className="ml-1.5 inline-block max-w-full truncate rounded bg-surface-subtle px-1 py-px align-middle font-mono text-[11px] text-content-muted">
                        {step.config_hint}
                      </span>
                    ) : null}
                  </li>
                ))}
              </ol>
            ) : (
              <p className="mt-1 text-xs text-content-faint">{t('chat.flowProposal.noSteps')}</p>
            )}
          </div>

          {proposal.requireApproval && (
            <p className="mt-2 text-xs text-content-faint">
              {t('chat.flowProposal.requireApprovalHint')}
            </p>
          )}

          {errorMsg && <p className="mt-2 text-xs text-coral">⚠ {errorMsg}</p>}

          <div className="mt-3 flex flex-wrap items-center gap-2">
            <Button
              variant="primary"
              size="sm"
              data-analytics-id="workflow-proposal-save"
              onClick={() => void save()}
              disabled={saving}>
              {saving ? t('chat.flowProposal.saving') : t('chat.flowProposal.save')}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              data-analytics-id="workflow-proposal-open-canvas"
              onClick={openInCanvas}
              disabled={saving}>
              {t('chat.flowProposal.openInCanvas')}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              data-analytics-id="workflow-proposal-dismiss"
              onClick={dismiss}
              disabled={saving}>
              {t('chat.flowProposal.dismiss')}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
};

export default WorkflowProposalCard;
