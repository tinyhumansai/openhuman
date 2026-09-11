import type { ReactNode } from 'react';

import { Spinner } from './icons';

/**
 * The app's spinner, re-exported here so loading UI has one import.
 *
 * There is deliberately no `ui/Spinner.tsx`: the SVG already lives in
 * `ui/icons.tsx` (`animate-spin`, `currentColor`, sized by `className`), and a
 * second spinner component would be a second thing to keep in sync for no new
 * behaviour. Size it the way every other icon in this repo is sized —
 * `<Spinner className="h-5 w-5 text-content-muted" />`.
 */
export { Spinner };

export interface ErrorBannerProps {
  children?: ReactNode;
  message?: ReactNode;
  action?: ReactNode;
  size?: 'sm' | 'md';
  className?: string;
}

const ERROR_BANNER_SIZES = { sm: 'p-3 text-xs', md: 'p-4 text-sm' } as const;

export function ErrorBanner({
  children,
  message,
  action,
  size = 'sm',
  className,
}: ErrorBannerProps) {
  return (
    <div
      role="alert"
      className={`rounded-xl border border-coral-500/20 bg-coral-500/10 text-coral-400 ${ERROR_BANNER_SIZES[size]} ${className ?? ''}`}>
      <div className={action ? 'flex items-start justify-between gap-3' : undefined}>
        <div className="min-w-0 flex-1">{children ?? message}</div>
        {action ? <div className="shrink-0">{action}</div> : null}
      </div>
    </div>
  );
}

interface InlineLoadingStatusProps {
  label: string;
  className?: string;
}

export function InlineLoadingStatus({ label, className }: InlineLoadingStatusProps) {
  return (
    <div className={`flex items-center gap-2 px-1 py-2 text-xs text-amber-400 ${className ?? ''}`}>
      <Spinner className="w-3 h-3" />
      {label}
    </div>
  );
}

interface CenteredLoadingStateProps {
  label?: string;
  className?: string;
}

export function CenteredLoadingState({ label, className }: CenteredLoadingStateProps) {
  return (
    <div className={`flex items-center justify-center py-8 ${className ?? ''}`}>
      <Spinner className="w-5 h-5 text-content-muted" />
      {label ? <span className="ml-3 text-sm text-content-muted">{label}</span> : null}
    </div>
  );
}
