import { Spinner } from './icons';

interface ErrorBannerProps {
  message: string;
  className?: string;
}

export function ErrorBanner({ message, className }: ErrorBannerProps) {
  return (
    <div className={`rounded-xl bg-coral-500/10 border border-coral-500/20 p-3 ${className ?? ''}`}>
      <p className="text-xs text-coral-400">{message}</p>
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
