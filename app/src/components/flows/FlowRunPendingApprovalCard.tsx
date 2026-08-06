/**
 * FlowRunPendingApprovalCard (flow-approval surface — run details)
 * ------------------------------------------------------------------
 *
 * Actionable replacement for the old read-only "N node(s) awaiting approval"
 * banner in `FlowRunInspectorDrawer`. Renders one gate from
 * `useFlowPendingApprovals` with Approve once / Approve always / Deny,
 * routing every decision through `openhuman.approval_decide` (same RPC and
 * decision vocabulary as the chat `ApprovalRequestCard`). Styling mirrors
 * that card's amber warning chrome, scaled down for the drawer's narrower
 * column.
 */
import { useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { type ApprovalDecision, type PendingApproval } from '../../services/api/approvalApi';
import ApprovalDecisionCard, {
  type ApprovalDecisionAction,
} from '../approvals/ApprovalDecisionCard';

interface Props {
  approval: PendingApproval;
  /** Whether THIS approval's decision RPC is currently in flight. */
  deciding: boolean;
  onDecide: (decision: ApprovalDecision) => Promise<void>;
}

export function FlowRunPendingApprovalCard({ approval, deciding, onDecide }: Props) {
  const { t } = useT();
  const [localDecision, setLocalDecision] = useState<ApprovalDecision | null>(null);

  const actionDecisions: Record<string, ApprovalDecision> = {
    [`flow-run-pending-approval-approve-${approval.request_id}`]: 'approve_once',
    [`flow-run-pending-approval-always-${approval.request_id}`]: 'approve_always_for_flow',
    [`flow-run-pending-approval-deny-${approval.request_id}`]: 'deny',
  };
  const actions: ApprovalDecisionAction[] = [
    {
      id: `flow-run-pending-approval-approve-${approval.request_id}`,
      label: t('flowRuns.inspector.approval.approve'),
      busyLabel: t('flowRuns.inspector.approval.deciding'),
      variant: 'primary',
    },
    {
      id: `flow-run-pending-approval-always-${approval.request_id}`,
      label: t('flowRuns.inspector.approval.approveAlways'),
      busyLabel: t('flowRuns.inspector.approval.deciding'),
      variant: 'secondary',
      title: t('flowRuns.inspector.approval.approveAlwaysHint'),
    },
    {
      id: `flow-run-pending-approval-deny-${approval.request_id}`,
      label: t('flowRuns.inspector.approval.deny'),
      busyLabel: t('flowRuns.inspector.approval.deciding'),
      variant: 'secondary',
    },
  ];

  const handleAction = (actionId: string) => {
    if (deciding) return;
    const decision = actionDecisions[actionId];
    if (!decision) return;
    setLocalDecision(decision);
    void onDecide(decision).catch(() => {
      // Error surfaces via the hook's shared `error` field; nothing extra to
      // do here besides letting the buttons re-enable (`deciding` flips back
      // to false on the parent's next render).
    });
  };

  const busyActionId = deciding
    ? localDecision
      ? actions.find(action => actionDecisions[action.id] === localDecision)?.id
      : `flow-run-pending-approval-busy-${approval.request_id}`
    : null;

  return (
    <ApprovalDecisionCard
      ariaLabel={t('flowRuns.inspector.pendingApprovals')}
      testId={`flow-run-pending-approval-${approval.request_id}`}
      className="text-xs"
      density="compact"
      summary={
        <p className="break-words text-amber-800/90 dark:text-amber-200/90">
          {approval.action_summary}
        </p>
      }
      metadata={
        <p className="mt-1 text-[11px] text-amber-800/80 dark:text-amber-200/80">
          {t('flowRuns.inspector.approval.tool')}{' '}
          <span className="font-mono text-amber-950 dark:text-amber-100">{approval.tool_name}</span>
        </p>
      }
      actions={actions}
      busyActionId={busyActionId}
      onAction={handleAction}
    />
  );
}
