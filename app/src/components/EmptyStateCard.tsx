import type { ReactNode } from 'react';

interface EmptyStateCardProps {
  icon: ReactNode;
  title: string;
  description: string;
  actionLabel?: string;
  onAction?: () => void;
  footer?: ReactNode;
  className?: string;
}

const EmptyStateCard = ({
  icon,
  title,
  description,
  actionLabel,
  onAction,
  footer,
  className = '',
}: EmptyStateCardProps) => {
  return (
    <div
      className={`flex flex-col items-center justify-center rounded-2xl border border-dashed border-line-strong bg-surface-muted/80 dark:bg-surface/80 px-6 py-16 text-center ${className}`.trim()}>
      <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-2xl bg-primary-50 dark:bg-primary-500/10">
        {icon}
      </div>
      <h3 className="text-base font-semibold text-content">{title}</h3>
      <p className="mt-2 max-w-md text-sm leading-relaxed text-content-muted">{description}</p>
      {actionLabel && onAction ? (
        <button
          type="button"
          onClick={onAction}
          className="mt-4 inline-flex items-center gap-1.5 rounded-full bg-primary-50 dark:bg-primary-500/10 px-3 py-1.5 text-xs font-medium text-primary-600 dark:text-primary-400 border border-primary-100 dark:border-primary-800/50 transition-colors hover:bg-primary-100 dark:hover:bg-primary-500/20">
          <span>{actionLabel}</span>
          <svg
            className="h-3.5 w-3.5"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={2}
            aria-hidden="true">
            <path strokeLinecap="round" strokeLinejoin="round" d="M9 5l7 7-7 7" />
          </svg>
        </button>
      ) : null}
      {footer}
    </div>
  );
};

export default EmptyStateCard;
