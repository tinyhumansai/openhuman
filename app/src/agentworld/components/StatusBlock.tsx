import type { ReactNode } from 'react';

import { Spinner } from '../../components/ui';

export type StatusBlockTone = 'neutral' | 'info' | 'success' | 'warning' | 'danger';

export interface StatusBlockProps {
  tone?: StatusBlockTone;
  title: ReactNode;
  body?: ReactNode;
  loading?: boolean;
  action?: ReactNode;
  className?: string;
}

const TONE_CLASSES: Record<StatusBlockTone, string> = {
  neutral: 'text-content-muted',
  info: 'text-ocean-600 dark:text-ocean-300',
  success: 'text-sage-600 dark:text-sage-300',
  warning: 'text-amber-600 dark:text-amber-400',
  danger: 'text-coral-500',
};

export function StatusBlock({
  tone = 'neutral',
  title,
  body,
  loading = false,
  action,
  className,
}: StatusBlockProps) {
  return (
    <div
      aria-busy={loading || undefined}
      className={`flex h-64 flex-col items-center justify-center gap-2 text-center ${className ?? ''}`}
      data-testid="agentworld-status-block">
      {loading ? (
        <Spinner aria-label="Loading" className="h-5 w-5 text-content-muted" role="status" />
      ) : null}
      <p className={`text-base font-medium ${TONE_CLASSES[tone]}`}>{title}</p>
      {body != null ? <p className="max-w-md text-sm text-content-muted">{body}</p> : null}
      {action != null ? <div className="mt-2">{action}</div> : null}
    </div>
  );
}

export default StatusBlock;
