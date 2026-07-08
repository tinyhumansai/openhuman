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
import Button from '../ui/Button';

interface Props {
  approval: PendingApproval;
  /** Whether THIS approval's decision RPC is currently in flight. */
  deciding: boolean;
  onDecide: (decision: ApprovalDecision) => Promise<void>;
}

export function FlowRunPendingApprovalCard({ approval, deciding, onDecide }: Props) {
  const { t } = useT();
  const [localDecision, setLocalDecision] = useState<ApprovalDecision | null>(null);

  const handleDecide = (decision: ApprovalDecision) => {
    if (deciding) return;
    setLocalDecision(decision);
    void onDecide(decision).catch(() => {
      // Error surfaces via the hook's shared `error` field; nothing extra to
      // do here besides letting the buttons re-enable (`deciding` flips back
      // to false on the parent's next render).
    });
  };

  return (
    <div
      role="alertdialog"
      aria-label={t('flowRuns.inspector.pendingApprovals')}
      data-testid={`flow-run-pending-approval-${approval.request_id}`}
      className="rounded-xl border border-amber-300 bg-amber-50 p-3 text-xs shadow-sm dark:border-amber-700 dark:bg-amber-950">
      <div className="flex items-start gap-2">
        <span aria-hidden className="text-sm leading-none">
          🔒
        </span>
        <div className="min-w-0 flex-1">
          <p className="break-words text-amber-800/90 dark:text-amber-200/90">
            {approval.action_summary}
          </p>
          <p className="mt-1 text-[11px] text-amber-800/80 dark:text-amber-200/80">
            {t('flowRuns.inspector.approval.tool')}{' '}
            <span className="font-mono text-amber-950 dark:text-amber-100">
              {approval.tool_name}
            </span>
          </p>

          <div className="mt-2 flex flex-wrap items-center gap-1.5">
            <Button
              variant="primary"
              size="xs"
              data-testid={`flow-run-pending-approval-approve-${approval.request_id}`}
              disabled={deciding}
              onClick={() => handleDecide('approve_once')}>
              {deciding && localDecision === 'approve_once'
                ? t('flowRuns.inspector.approval.deciding')
                : t('flowRuns.inspector.approval.approve')}
            </Button>
            <Button
              variant="secondary"
              size="xs"
              data-testid={`flow-run-pending-approval-always-${approval.request_id}`}
              disabled={deciding}
              title={t('flowRuns.inspector.approval.approveAlwaysHint')}
              onClick={() => handleDecide('approve_always_for_flow')}>
              {deciding && localDecision === 'approve_always_for_flow'
                ? t('flowRuns.inspector.approval.deciding')
                : t('flowRuns.inspector.approval.approveAlways')}
            </Button>
            <Button
              variant="secondary"
              size="xs"
              data-testid={`flow-run-pending-approval-deny-${approval.request_id}`}
              disabled={deciding}
              onClick={() => handleDecide('deny')}>
              {deciding && localDecision === 'deny'
                ? t('flowRuns.inspector.approval.deciding')
                : t('flowRuns.inspector.approval.deny')}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}

export default FlowRunPendingApprovalCard;
