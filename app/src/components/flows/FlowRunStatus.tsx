import { cn } from '../../lib/cn';
import type { Translate } from '../../lib/flows/cron';
import type { FlowRunStatus as FlowRunStatusValue } from '../../services/api/flowsApi';

export const FLOW_RUN_STATUS_ACCENT: Record<FlowRunStatusValue, string> = {
  running:
    'border-primary-200 bg-primary-50 text-primary-700 dark:border-primary-500/30 dark:bg-primary-500/10 dark:text-primary-300',
  completed:
    'border-sage-200 bg-sage-50 text-sage-700 dark:border-sage-500/30 dark:bg-sage-500/10 dark:text-sage-300',
  completed_with_warnings:
    'border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-300',
  pending_approval:
    'border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-300',
  failed:
    'border-coral-200 bg-coral-50 text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300',
  cancelled: 'border-line bg-surface-muted text-content-secondary',
  interrupted:
    'border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-300',
};

export const FLOW_RUN_STATUS_DOT: Record<FlowRunStatusValue, string> = {
  running: 'bg-primary-500 animate-pulse',
  completed: 'bg-sage-500',
  completed_with_warnings: 'bg-amber-500',
  pending_approval: 'bg-amber-500 animate-pulse',
  failed: 'bg-coral-500',
  cancelled: 'bg-surface-strong',
  interrupted: 'bg-amber-500',
};

export const FLOW_RUN_STATUS_KEY: Record<FlowRunStatusValue, string> = {
  running: 'flowRuns.status.running',
  completed: 'flowRuns.status.completed',
  completed_with_warnings: 'flowRuns.status.completed_with_warnings',
  pending_approval: 'flowRuns.status.pending_approval',
  failed: 'flowRuns.status.failed',
  cancelled: 'flowRuns.status.cancelled',
  interrupted: 'flowRuns.status.interrupted',
};

// Fallback styling/label for a wire status this build doesn't recognize.
// Run payloads come off the RPC as a cast, not a validated enum (`flowsApi.ts`),
// so a future/renamed `FlowRunStatus` the backend introduces must not index
// these `Record`s with an unknown key and render `undefined` — see F-m8.
const UNKNOWN_STATUS_ACCENT = 'border-line bg-surface-muted text-content-secondary';
const UNKNOWN_STATUS_DOT = 'bg-surface-strong';

/** Runtime-safe accent class lookup — falls back for an unrecognized status. */
export function flowRunStatusAccentClass(status: FlowRunStatusValue): string {
  return FLOW_RUN_STATUS_ACCENT[status] ?? UNKNOWN_STATUS_ACCENT;
}

/** Runtime-safe dot class lookup — falls back for an unrecognized status. */
export function flowRunStatusDotClass(status: FlowRunStatusValue): string {
  return FLOW_RUN_STATUS_DOT[status] ?? UNKNOWN_STATUS_DOT;
}

/**
 * Runtime-safe, localized label for a run status. Mirrors
 * `WorkflowRunsPage.tsx`'s `statusLabel` fallback pattern (an English
 * space-separated fallback passed as `t`'s second argument) rather than
 * indexing {@link FLOW_RUN_STATUS_KEY} directly and handing `t` a possibly
 * `undefined` key, which renders literally as "undefined" for a status this
 * build doesn't recognize yet.
 */
export function flowRunStatusLabel(status: FlowRunStatusValue, t: Translate): string {
  const fallback = status.replace(/_/g, ' ');
  const key = FLOW_RUN_STATUS_KEY[status];
  return key ? t(key, fallback) : fallback;
}

export type FlowRunStatusPresentation = 'badge' | 'dot';

export interface FlowRunStatusProps {
  status: FlowRunStatusValue;
  label: string;
  presentation?: FlowRunStatusPresentation;
  className?: string;
  testId?: string;
}

export function FlowRunStatus({
  status,
  label,
  presentation = 'badge',
  className = '',
  testId,
}: FlowRunStatusProps) {
  if (presentation === 'dot') {
    return (
      <span
        data-testid={testId}
        data-status={status}
        className={cn('h-2 w-2 shrink-0 rounded-full', flowRunStatusDotClass(status), className)}
        aria-hidden
      />
    );
  }

  return (
    <span
      data-testid={testId}
      data-status={status}
      className={cn(
        'inline-flex items-center rounded-full border px-2 py-0.5 font-medium',
        flowRunStatusAccentClass(status),
        className
      )}>
      {label}
    </span>
  );
}
