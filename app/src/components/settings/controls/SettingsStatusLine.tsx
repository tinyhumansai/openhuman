import { type ReactNode } from 'react';

export interface SettingsStatusLineProps {
  saving: boolean;
  savedNote?: string | null;
  error?: string | null;
  savingLabel: string;
  className?: string;
}

const SettingsStatusLine = ({
  saving,
  savedNote,
  error,
  savingLabel,
  className,
}: SettingsStatusLineProps) => {
  const wrapperClass = ['min-h-[1.25rem] text-xs', className ?? ''].filter(Boolean).join(' ');

  let content: ReactNode = null;

  if (error) {
    content = <span className="text-coral-600 dark:text-coral-300">{error}</span>;
  } else if (saving) {
    content = <span className="text-content-muted animate-pulse">{savingLabel}</span>;
  } else if (savedNote) {
    content = (
      <span className="text-sage-700 dark:text-sage-300 flex items-center gap-1">
        <svg
          aria-hidden="true"
          className="inline-block w-3 h-3 flex-shrink-0"
          viewBox="0 0 12 12"
          fill="none"
          xmlns="http://www.w3.org/2000/svg">
          <path
            d="M2 6l3 3 5-5"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
        {savedNote}
      </span>
    );
  }

  return (
    <div className={wrapperClass} aria-live="polite" aria-atomic="true">
      {content}
    </div>
  );
};

export default SettingsStatusLine;
