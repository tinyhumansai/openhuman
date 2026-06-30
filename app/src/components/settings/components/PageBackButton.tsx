import type { ReactNode } from 'react';

interface PageBackButtonProps {
  label: string;
  onClick: () => void;
  trailingContent?: ReactNode;
}

const PageBackButton = ({ label, onClick, trailingContent }: PageBackButtonProps) => {
  return (
    <div className="flex flex-wrap items-center justify-between gap-3">
      <button
        type="button"
        onClick={onClick}
        className="inline-flex items-center gap-2 rounded-full border border-line bg-surface px-4 py-2 text-sm font-semibold text-content-secondary shadow-sm transition-colors hover:bg-surface-hover dark:bg-surface-muted/60 dark:border-line-strong dark:bg-surface dark:text-neutral-200 dark:hover:bg-surface-muted">
        <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
        </svg>
        {label}
      </button>
      {trailingContent}
    </div>
  );
};

export default PageBackButton;
