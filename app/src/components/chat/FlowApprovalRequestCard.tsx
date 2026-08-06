/**
 * FlowApprovalRequestCard (flow-approval surface — chat)
 * ---------------------------------------------------------
 *
 * Chat-surfaced banner for a single `flow_approval_request` socket event
 * (see {@link useFlowApprovalRequests}) — a paused `tinyflows` run's gate,
 * shown to the user while they're chatting rather than inspecting the run
 * directly. Styling mirrors `ApprovalRequestCard` (the thread-scoped chat
 * tool-approval card) so both read as the same affordance family; unlike
 * that card, this one isn't keyed to the active thread — the payload has no
 * `thread_id` — so it renders independent of which thread is selected.
 *
 * All three decisions route through the shared `openhuman.approval_decide`
 * RPC via {@link decideApproval}. On success (or once the request no longer
 * needs surfacing) the parent removes it from its list via `onResolved`.
 */
import debug from 'debug';
import React, { useState } from 'react';

import type { FlowApprovalRequest } from '../../hooks/useFlowApprovalRequests';
import { useT } from '../../lib/i18n/I18nContext';
import { type ApprovalDecision, decideApproval } from '../../services/api/approvalApi';
import ApprovalDecisionCard, {
  type ApprovalDecisionAction,
} from '../approvals/ApprovalDecisionCard';

const log = debug('openhuman:chat:flow-approval-card');

interface Props {
  request: FlowApprovalRequest;
  onResolved: (requestId: string) => void;
}

export const FlowApprovalRequestCard: React.FC<Props> = ({ request, onResolved }) => {
  const { t } = useT();
  const [deciding, setDeciding] = useState<ApprovalDecision | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const actionDecisions: Record<string, ApprovalDecision> = {
    'flow-approval-request-approve': 'approve_once',
    'flow-approval-request-always': 'approve_always_for_flow',
    'flow-approval-request-deny': 'deny',
  };
  const actions: ApprovalDecisionAction[] = [
    {
      id: 'flow-approval-request-approve',
      label: t('chat.flowApproval.approve'),
      busyLabel: t('chat.flowApproval.deciding'),
      variant: 'primary',
    },
    {
      id: 'flow-approval-request-always',
      label: t('chat.flowApproval.approveAlways'),
      busyLabel: t('chat.flowApproval.deciding'),
      variant: 'secondary',
      title: t('chat.flowApproval.approveAlwaysHint'),
    },
    {
      id: 'flow-approval-request-deny',
      label: t('chat.flowApproval.deny'),
      busyLabel: t('chat.flowApproval.deciding'),
      variant: 'secondary',
    },
  ];

  const decide = async (decision: ApprovalDecision) => {
    if (deciding) return;
    setDeciding(decision);
    setErrorMsg(null);
    try {
      await decideApproval(request.request_id, decision);
      log('decide: ok request=%s decision=%s', request.request_id, decision);
      onResolved(request.request_id);
    } catch (err) {
      log('decide: failed request=%s err=%o', request.request_id, err);
      setErrorMsg(t('chat.flowApproval.error'));
      setDeciding(null);
    }
  };

  const busyActionId = deciding
    ? actions.find(action => actionDecisions[action.id] === deciding)?.id
    : null;

  return (
    <ApprovalDecisionCard
      ariaLabel={t('chat.flowApproval.title')}
      testId="flow-approval-request-card"
      className="text-sm"
      summary={
        <>
          <p className="font-semibold text-amber-900 dark:text-amber-100">
            {t('chat.flowApproval.title')}
          </p>
          <p className="mt-1 break-words text-amber-800/90 dark:text-amber-200/90">
            {request.summary || t('chat.flowApproval.fallback')}
          </p>
        </>
      }
      metadata={
        <>
          <p className="mt-1 text-xs text-amber-800/80 dark:text-amber-200/80">
            {t('chat.flowApproval.tool')}{' '}
            <span className="font-mono text-amber-950 dark:text-amber-100">
              {request.tool_name}
            </span>
          </p>
          <p className="mt-0.5 text-xs text-amber-800/80 dark:text-amber-200/80">
            {t('chat.flowApproval.flow')}{' '}
            <span className="font-mono text-amber-950 dark:text-amber-100">{request.flow_id}</span>
          </p>

          {errorMsg && <p className="mt-2 text-xs text-coral">⚠ {errorMsg}</p>}
        </>
      }
      actions={actions}
      busyActionId={busyActionId}
      onAction={actionId => {
        const decision = actionDecisions[actionId];
        if (decision) void decide(decision);
      }}
    />
  );
};
