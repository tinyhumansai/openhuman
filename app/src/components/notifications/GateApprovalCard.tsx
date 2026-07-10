/**
 * GateApprovalCard (flow-approval surface — notifications)
 * ------------------------------------------------------------
 *
 * Approval surface for the `flow-gate-approval` CoreNotification kind — a
 * paused `tinyflows` run's approval gate, surfaced in the Notification
 * Center. `NotificationCenter` routes any notification matching
 * {@link isGateApprovalNotification} here instead of the generic
 * `CoreNotificationCard`.
 *
 * Distinct from the older `FlowApprovalCard` (`flow-pending-approval:`-id
 * notifications, issue B3a): that card resumes the run via
 * `openhuman.flows_resume` naming specific node ids. This one decides a
 * single gate via the shared `openhuman.approval_decide` RPC — the same
 * decision vocabulary as the chat `ApprovalRequestCard` and the flow-run
 * inspector's actionable cards (Approve once / Approve always / Deny).
 *
 * Payload shape (`notification.actions[0].payload`):
 * `{ request_id, flow_id, tool_name, summary }`.
 */
import debug from 'debug';
import { useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { type ApprovalDecision, decideApproval } from '../../services/api/approvalApi';
import { useAppDispatch } from '../../store/hooks';
import {
  clearNotificationActions,
  markRead,
  type NotificationItem,
} from '../../store/notificationSlice';
import Button from '../ui/Button';

const log = debug('notifications:gate-approval-card');

/** Shape of `notification.actions[0].payload` for the `flow-gate-approval` kind. */
interface GateApprovalPayload {
  request_id: string;
  flow_id: string;
  tool_name: string;
  summary: string;
}

function isGateApprovalPayload(value: unknown): value is GateApprovalPayload {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.request_id === 'string' &&
    typeof record.flow_id === 'string' &&
    typeof record.tool_name === 'string' &&
    typeof record.summary === 'string'
  );
}

/**
 * Matches a `NotificationItem` that should render as {@link GateApprovalCard}
 * — either the explicit `kind` field or (as a defensive fallback, mirroring
 * the older `flow-pending-approval:` id-prefix convention) an id starting
 * with `flow-gate-approval:`.
 */
export function isGateApprovalNotification(item: Pick<NotificationItem, 'id' | 'kind'>): boolean {
  return item.kind === 'flow-gate-approval' || item.id.startsWith('flow-gate-approval:');
}

interface Props {
  notification: NotificationItem;
}

const GateApprovalCard = ({ notification: n }: Props) => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const [deciding, setDeciding] = useState<ApprovalDecision | null>(null);
  const [error, setError] = useState<string | null>(null);

  const payload = n.actions?.[0]?.payload;
  const parsed = isGateApprovalPayload(payload) ? payload : null;

  const handleDecide = async (decision: ApprovalDecision) => {
    if (deciding) return;
    if (!parsed) {
      // Defensive — the core always stamps this shape, but never crash the
      // notification center on an unexpected payload.
      log('decide: missing/invalid payload notification=%s', n.id);
      setError(t('notifications.flowGate.error'));
      return;
    }
    setDeciding(decision);
    setError(null);
    log('decide: request=%s decision=%s', parsed.request_id, decision);
    try {
      await decideApproval(parsed.request_id, decision);
      log('decide: ok request=%s', parsed.request_id);
      dispatch(markRead({ id: n.id }));
      dispatch(clearNotificationActions({ id: n.id }));
    } catch (err) {
      log('decide: failed request=%s err=%o', parsed.request_id, err);
      setError(t('notifications.flowGate.error'));
    } finally {
      setDeciding(null);
    }
  };

  return (
    <div
      role="alertdialog"
      aria-label={t('notifications.flowGate.title')}
      data-testid="gate-approval-card"
      className="rounded-xl border border-amber-300 bg-amber-50 p-3 text-sm shadow-sm dark:border-amber-700 dark:bg-amber-950">
      <div className="flex items-start gap-2">
        <span aria-hidden className="text-base leading-none">
          🔒
        </span>
        <div className="min-w-0 flex-1">
          <p className="font-semibold text-amber-900 dark:text-amber-100">
            {t('notifications.flowGate.title')}
          </p>
          {(parsed?.summary || n.body) && (
            <p className="mt-1 break-words text-amber-800/90 dark:text-amber-200/90">
              {parsed?.summary || n.body}
            </p>
          )}
          {parsed && (
            <p className="mt-1 text-xs text-amber-800/80 dark:text-amber-200/80">
              {t('notifications.flowGate.tool')}{' '}
              <span className="font-mono text-amber-950 dark:text-amber-100">
                {parsed.tool_name}
              </span>
            </p>
          )}

          {error && <p className="mt-2 text-xs text-coral">⚠ {error}</p>}

          <div className="mt-3 flex flex-wrap items-center gap-2">
            <Button
              variant="primary"
              size="sm"
              data-testid="gate-approval-approve"
              disabled={deciding !== null}
              onClick={() => {
                void handleDecide('approve_once');
              }}>
              {deciding === 'approve_once'
                ? t('notifications.flowGate.deciding')
                : t('notifications.flowGate.approve')}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              data-testid="gate-approval-always"
              disabled={deciding !== null}
              title={t('notifications.flowGate.approveAlwaysHint')}
              onClick={() => {
                void handleDecide('approve_always_for_flow');
              }}>
              {deciding === 'approve_always_for_flow'
                ? t('notifications.flowGate.deciding')
                : t('notifications.flowGate.approveAlways')}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              data-testid="gate-approval-deny"
              disabled={deciding !== null}
              onClick={() => {
                void handleDecide('deny');
              }}>
              {deciding === 'deny'
                ? t('notifications.flowGate.deciding')
                : t('notifications.flowGate.deny')}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
};

export default GateApprovalCard;
