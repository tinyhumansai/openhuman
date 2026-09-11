import debug from 'debug';
import React, { useState } from 'react';

import Button from '../../../components/ui/Button';
import { useT } from '../../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../../services/coreRpcClient';
import {
  clearPendingPlanReviewForThread,
  type PendingPlanReview,
} from '../../../store/chatRuntimeSlice';
import { useAppDispatch } from '../../../store/hooks';

/**
 * Plan-mode review surface (Codex/Claude-style). The orchestrator parked the
 * live turn on a thread-scoped plan via the `request_plan_review` gate; this
 * card surfaces the plan above the composer and resolves the parked turn via
 * the `openhuman.plan_review_decide` RPC:
 *
 *  - **Approve & run** → the turn resumes and executes the plan.
 *  - **Reject** → the turn resumes and stops without executing.
 *  - **Send feedback** → the turn resumes, re-plans from the free-text request,
 *    and re-parks for another review.
 *
 * Mirrors {@link ApprovalRequestCard}: it owns the decision RPC and clears
 * itself optimistically; {@link ChatRuntimeProvider}'s turn-end handlers also
 * clear the pending review if the turn ends.
 */
const log = debug('openhuman:chat:plan-review-card');

type Decision = 'approve' | 'reject' | 'revise';

interface Props {
  threadId: string;
  review: PendingPlanReview;
}

export const PlanReviewCard: React.FC<Props> = ({ threadId, review }) => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const [feedback, setFeedback] = useState('');
  const [deciding, setDeciding] = useState<Decision | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const decide = async (decision: Decision, feedbackText?: string) => {
    if (deciding) return;
    setDeciding(decision);
    setErrorMsg(null);
    try {
      await callCoreRpc({
        method: 'openhuman.plan_review_decide',
        params: { request_id: review.requestId, decision, feedback: feedbackText },
      });
      // Resolve optimistically; ChatRuntimeProvider also clears on turn end.
      dispatch(clearPendingPlanReviewForThread({ threadId }));
    } catch (e) {
      log('plan_review_decide failed: %o', e);
      setErrorMsg(t('chat.approval.error'));
      setDeciding(null);
    }
  };

  const submitFeedback = () => {
    const trimmed = feedback.trim();
    if (!trimmed) return;
    void decide('revise', trimmed);
  };

  return (
    <div
      role="alertdialog"
      aria-label={t('conversations.planReview.title')}
      data-testid="plan-review-card"
      className="mb-2 rounded-xl border border-primary-300 bg-surface p-3 text-sm shadow-md dark:border-primary-700">
      <div className="flex items-start gap-2">
        <span aria-hidden className="text-base leading-none text-primary-700 dark:text-primary-200">
          🗺️
        </span>
        <div className="min-w-0 flex-1">
          <p className="font-semibold text-primary-900 dark:text-primary-100">
            {t('conversations.planReview.title')}
          </p>
          <p className="mt-1 wrap-break-word text-primary-800/90 dark:text-primary-200/90">
            {review.summary?.trim() || t('conversations.planReview.subtitle')}
          </p>

          {review.steps.length > 0 && (
            <ol className="mt-2 max-h-56 list-decimal overflow-y-auto pl-6 text-content-secondary">
              {review.steps.map((step, i) => (
                <li key={i} className="wrap-break-word">
                  {step}
                </li>
              ))}
            </ol>
          )}

          {errorMsg && (
            <p className="mt-2 text-xs text-coral-600 dark:text-coral-400">⚠ {errorMsg}</p>
          )}

          <div className="mt-3 flex flex-wrap items-center gap-2">
            <Button
              variant="primary"
              size="sm"
              data-analytics-id="plan-review-approve"
              onClick={() => void decide('approve')}
              disabled={deciding !== null}>
              {deciding === 'approve'
                ? t('chat.approval.deciding')
                : t('conversations.planReview.approve')}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              data-analytics-id="plan-review-reject"
              onClick={() => void decide('reject')}
              disabled={deciding !== null}>
              {deciding === 'reject'
                ? t('chat.approval.deciding')
                : t('conversations.planReview.reject')}
            </Button>
          </div>

          <div className="mt-3">
            <label
              htmlFor="plan-review-feedback"
              className="mb-1 block text-xs font-medium text-primary-800/80 dark:text-primary-200/80">
              {t('conversations.planReview.feedbackLabel')}
            </label>
            <textarea
              id="plan-review-feedback"
              data-testid="plan-review-feedback"
              value={feedback}
              onChange={e => setFeedback(e.target.value)}
              onKeyDown={e => {
                if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
                  e.preventDefault();
                  submitFeedback();
                }
              }}
              rows={2}
              disabled={deciding !== null}
              placeholder={t('conversations.planReview.feedbackPlaceholder')}
              className="w-full resize-y rounded-lg border border-primary-200 bg-surface px-2.5 py-1.5 text-sm text-content shadow-inner outline-hidden focus:border-primary-400 disabled:opacity-50 dark:border-primary-800 dark:bg-surface-canvas"
            />
            <div className="mt-1.5 flex justify-end">
              <Button
                variant="secondary"
                size="sm"
                data-analytics-id="plan-review-send-feedback"
                onClick={submitFeedback}
                disabled={deciding !== null || feedback.trim().length === 0}>
                {deciding === 'revise'
                  ? t('chat.approval.deciding')
                  : t('conversations.planReview.sendFeedback')}
              </Button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};
