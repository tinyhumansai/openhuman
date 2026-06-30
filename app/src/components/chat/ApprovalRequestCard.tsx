import debug from 'debug';
import React, { useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../services/coreRpcClient';
import { clearPendingApprovalForThread, type PendingApproval } from '../../store/chatRuntimeSlice';
import { useAppDispatch } from '../../store/hooks';
import Button from '../ui/Button';

/**
 * Decision surface for a parked tool call. `approve_once` / `deny` decide the
 * current call only; `approve_always_for_tool` additionally persists the tool
 * onto the user's `autonomy.auto_approve` ("Always allow") list so the gate
 * skips prompting for it on future turns (managed/removable in Settings → Agent
 * access). A typed `yes`/`no` chat reply is the equivalent server-side path for
 * the once/deny decisions.
 */
const log = debug('openhuman:chat:approval-card');

type Decision = 'approve_once' | 'approve_always_for_tool' | 'deny';

interface Props {
  threadId: string;
  approval: PendingApproval;
}

/**
 * Surfaces a `Prompt`-class tool call parked on the ApprovalGate
 * (`approval_request` socket event) and routes the user's Approve / Deny to the
 * `openhuman.approval_decide` RPC. Rendered above the composer for the active
 * thread; clears itself on a recorded decision (the turn-end handlers in
 * {@link ChatRuntimeProvider} also clear it if the turn is cancelled).
 */
export const ApprovalRequestCard: React.FC<Props> = ({ threadId, approval }) => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const [deciding, setDeciding] = useState<Decision | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const decide = async (decision: Decision) => {
    if (deciding) return;
    setDeciding(decision);
    setErrorMsg(null);
    try {
      await callCoreRpc({
        method: 'openhuman.approval_decide',
        params: { request_id: approval.requestId, decision },
      });
      // Resolve optimistically; ChatRuntimeProvider also clears on turn end.
      dispatch(clearPendingApprovalForThread({ threadId }));
    } catch (e) {
      // Keep raw RPC error detail in namespaced dev logs only; show the user the
      // localized fallback — never leak internal error text into the UI.
      log('approval_decide failed: %o', e);
      setErrorMsg(t('chat.approval.error'));
      setDeciding(null);
    }
  };

  return (
    <div
      role="alertdialog"
      aria-label={t('chat.approval.title')}
      className="rounded-xl border border-amber-300 bg-amber-50 p-3 text-sm shadow-sm dark:border-amber-700 dark:bg-amber-950">
      <div className="flex items-start gap-2">
        <span aria-hidden className="text-base leading-none text-amber-700 dark:text-amber-200">
          🔒
        </span>
        <div className="min-w-0 flex-1">
          <p className="font-semibold text-amber-900 dark:text-amber-100">
            {t('chat.approval.title')}
          </p>
          <p className="mt-1 break-words text-amber-800/90 dark:text-amber-200/90">
            {approval.message || t('chat.approval.fallback')}
          </p>
          {approval.command && (
            <pre className="mt-2 max-h-40 overflow-auto whitespace-pre-wrap break-all rounded border border-amber-200/80 bg-surface px-2 py-1.5 font-mono text-xs text-ink shadow-inner dark:border-amber-700 dark:bg-surface-canvas dark:text-content">
              {approval.command}
            </pre>
          )}
          <p className="mt-1 text-xs text-amber-800/80 dark:text-amber-200/80">
            {t('chat.approval.tool')}{' '}
            <span className="font-mono text-amber-950 dark:text-amber-100">
              {approval.toolName}
            </span>
          </p>

          {errorMsg && <p className="mt-2 text-xs text-coral">⚠ {errorMsg}</p>}

          <div className="mt-3 flex flex-wrap items-center gap-2">
            <Button
              variant="primary"
              size="sm"
              data-analytics-id="chat-approval-approve-once"
              onClick={() => void decide('approve_once')}
              disabled={deciding !== null}>
              {deciding === 'approve_once'
                ? t('chat.approval.deciding')
                : t('chat.approval.approve')}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              data-analytics-id="chat-approval-approve-always"
              onClick={() => void decide('approve_always_for_tool')}
              disabled={deciding !== null}
              title={t('chat.approval.alwaysAllowHint')}>
              {deciding === 'approve_always_for_tool'
                ? t('chat.approval.deciding')
                : t('chat.approval.alwaysAllow')}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              data-analytics-id="chat-approval-deny"
              onClick={() => void decide('deny')}
              disabled={deciding !== null}>
              {deciding === 'deny' ? t('chat.approval.deciding') : t('chat.approval.deny')}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
};

export default ApprovalRequestCard;
