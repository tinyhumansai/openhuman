/**
 * FlowRunPendingApprovalCard (flow-approval surface — run details)
 * ------------------------------------------------------------------
 *
 * Thin caller of the shared `ApprovalCardAdapter` (assistant-ui-elements
 * plan, WS-B row) for one gate from `useFlowPendingApprovals`, rendered in
 * `FlowRunInspectorDrawer`. Approve once / Approve always / Deny, routing
 * every decision through `openhuman.approval_decide` (same RPC and decision
 * vocabulary as every other approval surface).
 */
import { ApprovalCardAdapter } from '../../features/conversations/aui/ApprovalCardAdapter';
import { useT } from '../../lib/i18n/I18nContext';
import { type ApprovalDecisionInput, type PendingApproval } from '../../services/api/approvalApi';

interface Props {
  approval: PendingApproval;
  /** Whether THIS approval's decision RPC is currently in flight. */
  deciding: boolean;
  onDecide: (decision: ApprovalDecisionInput) => Promise<void>;
}

export function FlowRunPendingApprovalCard({ approval, deciding, onDecide }: Props) {
  const { t } = useT();

  return (
    <ApprovalCardAdapter
      ariaLabel={t('flowRuns.inspector.pendingApprovals')}
      testId={`flow-run-pending-approval-${approval.request_id}`}
      title={t('flowRuns.inspector.pendingApprovals')}
      subtitle={approval.action_summary}
      command={approval.tool_name}
      toolName={approval.tool_name}
      expiresAt={approval.expires_at}
      alwaysDecision="approve_always_for_flow"
      alwaysHint={t('flowRuns.inspector.approval.approveAlwaysHint')}
      analyticsPrefix={`flow-run-pending-approval-${approval.request_id}`}
      busy={deciding}
      onDecide={onDecide}
    />
  );
}
