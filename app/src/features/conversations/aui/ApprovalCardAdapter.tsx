'use client';

/**
 * OpenHuman glue over the vendored `elements/approval-card.tsx`.
 *
 * Two call sites share this one adapter (per the assistant-ui-elements plan,
 * WS-B row: "Build ONE adapter component for this out-of-thread case"):
 *
 * - In-thread: `ChatToolParts.tsx`'s `GatedToolCall`, for a `Prompt`-class
 *   tool call parked on the ApprovalGate and attached to its own tool-call
 *   part (`approval_request` socket event). Replaces the deleted
 *   `ApprovalRequestCard`.
 * - Out-of-thread: the composer-header decks in `Conversations.tsx` (a
 *   paused `tinyflows` run's `flow_approval_request`, or a background park
 *   with no owning thread/run — `approval_list_pending`), and the flow-run
 *   inspector's `FlowRunPendingApprovalCard`. Replaces the deleted
 *   `FlowApprovalRequestCard` / `UnroutedApprovalCard` / `ApprovalDecisionCard`.
 *
 * Every decision still routes through the single shared
 * `openhuman.approval_decide` RPC (`services/api/approvalApi.ts`'s
 * `decideApproval`) — this component owns only the deciding/error UI state
 * and the vendored element's props, never the RPC itself; callers pass
 * `onDecide`.
 */
import debug from 'debug';
import { useState } from 'react';

import { ApprovalCard } from '../../../components/assistant-ui/elements/approval-card';
import { useT } from '../../../lib/i18n/I18nContext';
import type { ApprovalDecisionInput } from '../../../services/api/approvalApi';
import { formatCountdown, useApprovalExpirySeconds } from './approvalCountdown';

const log = debug('openhuman:aui:approval-card-adapter');

/**
 * `D` is the decision vocabulary sent to `onDecide` — the real
 * `openhuman.approval_decide` RPC's {@link ApprovalDecisionInput} for every
 * approval-gate call site (the default), or a different RPC's own decision
 * union for a call site whose "deny / always-allow / allow-once" SHAPE fits
 * this card even though its wire vocabulary doesn't (e.g. `PlanReviewPart`'s
 * `openhuman.plan_review_decide`, which sends `'approve' | 'reject' |
 * 'revise'`). The adapter never interprets `D` itself — it only forwards
 * whatever the caller passes as `alwaysDecision`/the fixed once/deny calls
 * below to `onDecide` — so a second vocabulary costs the caller nothing.
 */
export interface ApprovalCardAdapterProps<D = ApprovalDecisionInput> {
  ariaLabel: string;
  title: string;
  subtitle: string;
  /** The exact command/target rendered in the card's mono panel. */
  command: string;
  toolName: string;
  /** RFC3339 timestamp, or `null`/absent when the request does not expire. */
  expiresAt?: string | null;
  /**
   * Decision to send for "Always allow". Omit to hide that button entirely
   * (e.g. the unrouted-approval surface, which deliberately offers only
   * once/deny — see the deleted `UnroutedApprovalCard`'s doc comment).
   */
  // `NoInfer` keeps a plain string-literal decision (e.g.
  // `alwaysDecision="approve_always_for_tool"`) from narrowing `D` away from
  // its `ApprovalDecision` default at an ordinary approval-gate call site
  // that passes no explicit `<D>` — only an explicit type argument (like
  // `PlanReviewPart`'s `<ApprovalCardAdapter<Decision>>`) should do that.
  alwaysDecision?: NoInfer<D>;
  alwaysHint?: string;
  /**
   * Local UI action for "Always allow" instead of an `onDecide` dispatch —
   * e.g. `PlanReviewPart`'s "Revise" button, which opens a feedback textarea
   * rather than sending a decision immediately. Takes precedence over
   * `alwaysDecision` when both are given; does not enter the `deciding`
   * state (there is nothing pending to show "deciding" for).
   */
  onAlwaysAllowClick?: () => void;
  /** Decision to send for "Deny". Defaults to the approval-gate `'deny'`. */
  denyDecision?: NoInfer<D>;
  /** Decision to send for "Allow once". Defaults to the approval-gate `'approve_once'`. */
  allowOnceDecision?: NoInfer<D>;
  /** Button label overrides, for a call site whose vocabulary differs from the approval-gate's ("Reject" vs. "Deny", "Approve & run" vs. "Approve"). Default to the approval-gate copy. */
  denyLabel?: string;
  alwaysAllowLabel?: string;
  allowOnceLabel?: string;
  onDecide: (decision: D) => Promise<void>;
  /** Prefix for each button's `data-analytics-id` / e2e `data-testid`. */
  analyticsPrefix: string;
  testId?: string;
  className?: string;
  /** External busy flag (the unrouted deck shares one busy state across rows). */
  busy?: boolean;
}

export function ApprovalCardAdapter<D = ApprovalDecisionInput>({
  ariaLabel,
  title,
  subtitle,
  command,
  toolName,
  expiresAt,
  alwaysDecision,
  alwaysHint,
  onAlwaysAllowClick,
  denyDecision = 'deny' as D,
  allowOnceDecision = 'approve_once' as D,
  denyLabel,
  alwaysAllowLabel,
  allowOnceLabel,
  onDecide,
  analyticsPrefix,
  testId,
  className,
  busy = false,
}: ApprovalCardAdapterProps<D>) {
  const { t } = useT();
  const [deciding, setDeciding] = useState<D | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const expirySeconds = useApprovalExpirySeconds(expiresAt);

  const decide = async (decision: D) => {
    if (deciding || busy) return;
    setDeciding(decision);
    setErrorMsg(null);
    try {
      await onDecide(decision);
    } catch (e) {
      log('decide(%s) failed: %o', decision, e);
      setErrorMsg(t('chat.approval.error'));
      setDeciding(null);
    }
  };

  const disabled = deciding !== null || busy;

  return (
    <div role="alertdialog" aria-label={ariaLabel} data-testid={testId} className={className}>
      <ApprovalCard
        state={deciding ? 'running' : 'request'}
        title={title}
        subtitle={subtitle}
        command={command || toolName}
        expiry={
          expirySeconds !== null ? (
            <span className="text-foreground/35 mt-0.5 text-[11px] tabular-nums">
              {t('chat.approval.expiresIn').replace('{time}', formatCountdown(expirySeconds))}
            </span>
          ) : undefined
        }
        denyLabel={denyLabel ?? t('chat.approval.deny')}
        alwaysAllowLabel={alwaysAllowLabel ?? t('chat.approval.alwaysAllow')}
        allowOnceLabel={allowOnceLabel ?? t('chat.approval.approve')}
        runningLabel={t('chat.approval.deciding')}
        onDeny={() => void decide(denyDecision)}
        onAlwaysAllow={
          onAlwaysAllowClick
            ? onAlwaysAllowClick
            : alwaysDecision
              ? () => void decide(alwaysDecision)
              : undefined
        }
        onAllowOnce={() => void decide(allowOnceDecision)}
        denyProps={{ 'data-analytics-id': `${analyticsPrefix}-deny`, disabled }}
        alwaysAllowProps={
          onAlwaysAllowClick || alwaysDecision
            ? {
                'data-analytics-id': `${analyticsPrefix}-approve-always`,
                disabled: onAlwaysAllowClick ? deciding !== null || busy : disabled,
                title: alwaysHint,
              }
            : undefined
        }
        allowOnceProps={{ 'data-analytics-id': `${analyticsPrefix}-approve-once`, disabled }}
      />
      {errorMsg && (
        <p role="alert" className="mt-2 text-xs text-coral-600 dark:text-coral-400">
          ⚠ {errorMsg}
        </p>
      )}
    </div>
  );
}
