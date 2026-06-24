/**
 * WorkflowRunApprovalCard (#3375)
 * --------------------------------
 *
 * Confirmation surface shown before starting a high-cost / high-concurrency
 * workflow run. The acceptance criterion "High-cost/high-concurrency runs
 * require explicit approval" is enforced on the client: the Orchestration tab
 * calls {@link assessWorkflowCost} and, when it returns `requiresApproval`,
 * renders this card instead of starting immediately. The user must press
 * "Approve & start" to proceed; "Cancel" aborts without touching the engine.
 *
 * Styling mirrors the chat ApprovalRequestCard (amber warning chrome) so the
 * approval affordance is visually consistent across the app.
 */
import debug from 'debug';
import React from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type WorkflowCostReason,
  type WorkflowDefinition,
  type WorkflowSafetyTier,
} from '../../services/api/workflowRunsApi';
import Button from '../ui/Button';

const log = debug('intelligence:workflow-approval');

/** i18n key for each cost reason code. */
const REASON_KEY: Record<WorkflowCostReason, string> = {
  non_read_only_tier: 'orchestration.approval.reason.tier',
  high_concurrency: 'orchestration.approval.reason.concurrency',
  high_children: 'orchestration.approval.reason.children',
};

/** i18n key for each safety tier label. */
export const SAFETY_TIER_KEY: Record<WorkflowSafetyTier, string> = {
  read_only: 'orchestration.tier.readOnly',
  standard: 'orchestration.tier.standard',
  edit_capable: 'orchestration.tier.editCapable',
};

interface Props {
  definition: WorkflowDefinition;
  reasons: WorkflowCostReason[];
  /** Whether the start RPC is in flight (disables the buttons). */
  starting?: boolean;
  onApprove: () => void;
  onCancel: () => void;
}

export const WorkflowRunApprovalCard: React.FC<Props> = ({
  definition,
  reasons,
  starting = false,
  onApprove,
  onCancel,
}) => {
  const { t } = useT();

  return (
    <div
      role="alertdialog"
      aria-label={t('orchestration.approval.title')}
      data-testid="workflow-approval-card"
      className="rounded-xl border border-amber-300 bg-amber-50 p-4 text-sm shadow-sm dark:border-amber-700 dark:bg-amber-950">
      <div className="flex items-start gap-2">
        <span aria-hidden className="text-base leading-none">
          ⚠️
        </span>
        <div className="min-w-0 flex-1">
          <p className="font-semibold text-amber-900 dark:text-amber-200">
            {t('orchestration.approval.title')}
          </p>
          <p className="mt-1 break-words text-amber-800/90 dark:text-amber-200/90">
            {t('orchestration.approval.body')}{' '}
            <span className="font-semibold">{definition.name}</span>
          </p>

          {/* Why approval is required — one localized line per reason code. */}
          <ul
            data-testid="workflow-approval-reasons"
            className="mt-2 list-disc space-y-1 pl-5 text-xs text-amber-800/90 dark:text-amber-200/90">
            {reasons.map(reason => (
              <li key={reason}>{t(REASON_KEY[reason])}</li>
            ))}
          </ul>

          {/* Concrete cost facts so the user knows what they're approving. */}
          <dl className="mt-3 grid grid-cols-3 gap-2 text-xs">
            <div className="rounded-md bg-amber-100/70 px-2 py-1.5 dark:bg-amber-500/15">
              <dt className="text-amber-700/80 dark:text-amber-300/80">
                {t('orchestration.approval.tier')}
              </dt>
              <dd className="font-medium text-amber-900 dark:text-amber-100">
                {t(SAFETY_TIER_KEY[definition.safetyTier])}
              </dd>
            </div>
            <div className="rounded-md bg-amber-100/70 px-2 py-1.5 dark:bg-amber-500/15">
              <dt className="text-amber-700/80 dark:text-amber-300/80">
                {t('orchestration.approval.concurrency')}
              </dt>
              <dd className="font-medium text-amber-900 dark:text-amber-100">
                {definition.defaultConcurrency}
              </dd>
            </div>
            <div className="rounded-md bg-amber-100/70 px-2 py-1.5 dark:bg-amber-500/15">
              <dt className="text-amber-700/80 dark:text-amber-300/80">
                {t('orchestration.approval.maxChildren')}
              </dt>
              <dd className="font-medium text-amber-900 dark:text-amber-100">
                {definition.maxChildren}
              </dd>
            </div>
          </dl>

          <div className="mt-3 flex flex-wrap items-center gap-2">
            <Button
              variant="primary"
              size="sm"
              data-testid="workflow-approval-approve"
              disabled={starting}
              onClick={() => {
                log('approve definitionId=%s', definition.id);
                onApprove();
              }}>
              {starting
                ? t('orchestration.approval.starting')
                : t('orchestration.approval.approve')}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              data-testid="workflow-approval-cancel"
              disabled={starting}
              onClick={() => {
                log('cancel definitionId=%s', definition.id);
                onCancel();
              }}>
              {t('orchestration.approval.cancel')}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
};

export default WorkflowRunApprovalCard;
