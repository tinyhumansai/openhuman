import type { ReactNode } from 'react';

import Button from '../ui/Button';

export interface ApprovalDecisionAction {
  id: string;
  label: ReactNode;
  busyLabel?: ReactNode;
  variant: 'primary' | 'secondary';
  tone?: 'default' | 'danger';
  title?: string;
}

export interface ApprovalDecisionCardProps {
  ariaLabel: string;
  summary: ReactNode;
  metadata?: ReactNode;
  actions: ApprovalDecisionAction[];
  busyActionId?: string | null;
  onAction: (actionId: string) => void;
  testId?: string;
  className?: string;
  density?: 'default' | 'compact';
}

export function ApprovalDecisionCard({
  ariaLabel,
  summary,
  metadata,
  actions,
  busyActionId = null,
  onAction,
  testId,
  className,
  density = 'default',
}: ApprovalDecisionCardProps) {
  const compact = density === 'compact';

  return (
    <div
      role="alertdialog"
      aria-label={ariaLabel}
      data-testid={testId}
      className={[
        'rounded-xl border border-amber-300 bg-amber-50 p-3 shadow-sm',
        'dark:border-amber-700 dark:bg-amber-950',
        className,
      ]
        .filter(Boolean)
        .join(' ')}>
      <div className="flex items-start gap-2">
        <span
          aria-hidden
          className={
            compact
              ? 'text-sm leading-none'
              : 'text-base leading-none text-amber-700 dark:text-amber-200'
          }>
          🔒
        </span>
        <div className="min-w-0 flex-1">
          {summary}
          {metadata}

          <div
            className={
              compact
                ? 'mt-2 flex flex-wrap items-center gap-1.5'
                : 'mt-3 flex flex-wrap items-center gap-2'
            }>
            {actions.map(action => (
              <Button
                key={action.id}
                variant={action.variant}
                tone={action.tone}
                size={compact ? 'xs' : 'sm'}
                title={action.title}
                data-testid={action.id}
                disabled={busyActionId !== null}
                onClick={() => onAction(action.id)}>
                {busyActionId === action.id && action.busyLabel !== undefined
                  ? action.busyLabel
                  : action.label}
              </Button>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

export default ApprovalDecisionCard;
