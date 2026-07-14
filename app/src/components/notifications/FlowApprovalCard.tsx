/**
 * FlowApprovalCard (issue B3a)
 * ----------------------------
 *
 * Approval surface for a paused `tinyflows` run. When a flow run pauses on an
 * approval-gated node, the Rust side (`notify_pending_approval` in
 * `src/openhuman/flows/ops.rs`) publishes a `CoreNotification` whose id starts
 * with `"flow-pending-approval:"` and carries a single `"approve"` action with
 * `{ flow_id, thread_id, node_ids }` in its payload. `NotificationCenter`
 * routes any such notification here instead of the generic
 * `CoreNotificationCard`.
 *
 * Approve calls `openhuman.flows_resume` (via {@link resumeFlow}) naming the
 * pending node ids as the approvals; success clears the notification's
 * actions and marks it read. Dismiss is UI-only: there is no `flows_deny` /
 * cancel-run RPC yet (documented follow-up — the run stays parked
 * `pending_approval` server-side and can still be approved later from the run
 * history), so Dismiss just clears the prompt from the Notification Center
 * without touching the engine.
 *
 * Styling mirrors the existing amber approval chrome
 * (`WorkflowRunApprovalCard`) and the `role="alertdialog"` a11y pattern
 * (`ApprovalRequestCard`) so this reads as the same affordance family.
 *
 * "View run" (B3b) opens {@link FlowRunInspectorDrawer} for the run's status +
 * step timeline (run id === the payload's `thread_id`) without disturbing the
 * Approve/Dismiss flow above.
 */
import debug from 'debug';
import { useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../lib/i18n/I18nContext';
import { resumeFlow } from '../../services/api/flowsApi';
import { useAppDispatch } from '../../store/hooks';
import {
  clearNotificationActions,
  markRead,
  type NotificationItem,
} from '../../store/notificationSlice';
import { type FlowRepairRequest, FlowRunInspectorDrawer } from '../flows/FlowRunInspectorDrawer';
import Button from '../ui/Button';

const log = debug('notifications:flow-approval-card');

/** Shape of `notification.actions[0].payload` set by `notify_pending_approval`. */
interface FlowApprovalPayload {
  flow_id: string;
  thread_id: string;
  node_ids: string[];
}

function isFlowApprovalPayload(value: unknown): value is FlowApprovalPayload {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.flow_id === 'string' &&
    typeof record.thread_id === 'string' &&
    Array.isArray(record.node_ids) &&
    record.node_ids.every((x: unknown) => typeof x === 'string')
  );
}

interface Props {
  notification: NotificationItem;
}

/**
 * Renders the `flow-pending-approval:*` core notification with Approve /
 * Dismiss actions. See module doc above for the RPC + payload contract.
 */
const FlowApprovalCard = ({ notification: n }: Props) => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const [pending, setPending] = useState<'approve' | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [inspecting, setInspecting] = useState(false);

  // "Fix with agent" (issue B22) — a run opened from this notification can
  // fail with the gate never actually reached (or after it), so the same
  // repair action `FlowsPage`'s run-history drawer offers must be reachable
  // from here too. This card can render from anywhere in the app (it's a
  // `CoreNotification` surface), so — unlike `FlowRunsSidebar`, which is
  // always already on the flow's own canvas — always navigate there first.
  const handleFixWithAgent = (request: FlowRepairRequest) => {
    log('fix with agent: notification=%s flow=%s run=%s', n.id, request.flowId, request.runId);
    setInspecting(false);
    navigate(`/flows/${request.flowId}`, {
      state: {
        copilotRepair: {
          runId: request.runId,
          error: request.error,
          failingNodeIds: request.failingNodeIds,
        },
      },
    });
  };

  const payload = n.actions?.[0]?.payload;
  const parsed = isFlowApprovalPayload(payload) ? payload : null;

  const clearNotification = () => {
    dispatch(markRead({ id: n.id }));
    dispatch(clearNotificationActions({ id: n.id }));
  };

  const handleApprove = async () => {
    if (pending) return;
    if (!parsed) {
      // Defensive — the Rust side always stamps this shape, but never crash
      // the notification center on an unexpected payload.
      log('approve: missing/invalid payload notification=%s', n.id);
      setError(t('notifications.flow.error'));
      return;
    }
    setPending('approve');
    setError(null);
    log(
      'approve: request flowId=%s threadId=%s nodeIds=%o',
      parsed.flow_id,
      parsed.thread_id,
      parsed.node_ids
    );
    try {
      const result = await resumeFlow(parsed.flow_id, parsed.thread_id, parsed.node_ids);
      if (result.pending_approvals && result.pending_approvals.length > 0) {
        // Sequential gates: the run parked again on the next approval. The core
        // re-publishes a fresh notification under the SAME `flow-pending-approval:`
        // id with the new pending node ids, so clearing here would wipe that next
        // prompt. Leave it — the store already holds the updated notification, and
        // resetting `pending` (below) re-enables Approve for the next gate.
        log('approve: parked again pending=%o notification=%s', result.pending_approvals, n.id);
      } else {
        log('approve: ok notification=%s', n.id);
        clearNotification();
      }
    } catch (err) {
      log('approve: failed notification=%s err=%o', n.id, err);
      setError(t('notifications.flow.error'));
    } finally {
      setPending(null);
    }
  };

  const handleDismiss = () => {
    if (pending) return;
    // UI-only: no `flows_deny` / cancel-run RPC exists yet (documented
    // follow-up). The run stays parked `pending_approval` server-side; this
    // only hides the prompt from the Notification Center.
    log('dismiss: notification=%s (UI-only, no RPC)', n.id);
    clearNotification();
  };

  const gateCount = parsed?.node_ids.length ?? 0;
  const gateCountLabel = t('notifications.flow.gateCount').replace('{count}', String(gateCount));

  return (
    <div
      role="alertdialog"
      aria-label={t('notifications.flow.approveTitle')}
      data-testid="flow-approval-card"
      className="rounded-xl border border-amber-300 bg-amber-50 p-3 text-sm shadow-sm dark:border-amber-700 dark:bg-amber-950">
      <div className="flex items-start gap-2">
        <span aria-hidden className="text-base leading-none">
          ⚠️
        </span>
        <div className="min-w-0 flex-1">
          <p className="font-semibold text-amber-900 dark:text-amber-100">
            {t('notifications.flow.approveTitle')}
          </p>
          {n.body && (
            <p className="mt-1 break-words text-amber-800/90 dark:text-amber-200/90">{n.body}</p>
          )}
          {gateCount > 0 && (
            <p className="mt-1 text-xs text-amber-800/80 dark:text-amber-200/80">
              {gateCountLabel}
            </p>
          )}

          {error && <p className="mt-2 text-xs text-coral">{`⚠ ${error}`}</p>}

          <div className="mt-3 flex flex-wrap items-center gap-2">
            <Button
              variant="primary"
              size="sm"
              data-testid="flow-approval-approve"
              title={t('notifications.flow.approveHint')}
              disabled={pending !== null}
              onClick={() => {
                void handleApprove();
              }}>
              {pending === 'approve'
                ? t('notifications.flow.approving')
                : t('notifications.flow.approve')}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              data-testid="flow-approval-dismiss"
              title={t('notifications.flow.dismissHint')}
              disabled={pending !== null}
              onClick={handleDismiss}>
              {t('notifications.flow.dismiss')}
            </Button>
            {parsed && (
              <Button
                variant="tertiary"
                size="sm"
                data-testid="flow-approval-view-run"
                onClick={() => {
                  log(
                    'viewRun: opening drawer notification=%s threadId=%s',
                    n.id,
                    parsed.thread_id
                  );
                  setInspecting(true);
                }}>
                {t('notifications.flow.viewRun')}
              </Button>
            )}
          </div>
        </div>
      </div>
      {parsed && (
        <FlowRunInspectorDrawer
          runId={inspecting ? parsed.thread_id : null}
          onClose={() => setInspecting(false)}
          onFixWithAgent={handleFixWithAgent}
        />
      )}
    </div>
  );
};

export default FlowApprovalCard;
