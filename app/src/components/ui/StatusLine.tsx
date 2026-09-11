import { type ReactNode } from 'react';

import { cn } from '../../lib/cn';
import { CheckIcon } from './icons';

export interface StatusLineProps {
  saving: boolean;
  savedNote?: string | null;
  error?: string | null;
  savingLabel: string;
  className?: string;
}

/**
 * A polite live region for save state. Error wins over saving wins over saved,
 * so a failure is never masked by a stale success note.
 */
const StatusLine = ({ saving, savedNote, error, savingLabel, className }: StatusLineProps) => {
  let content: ReactNode = null;

  if (error) {
    content = <span className="text-coral-600 dark:text-coral-300">{error}</span>;
  } else if (saving) {
    content = <span className="animate-pulse text-content-muted">{savingLabel}</span>;
  } else if (savedNote) {
    content = (
      <span className="flex items-center gap-1 text-sage-700 dark:text-sage-300">
        <CheckIcon className="inline-block h-3 w-3 shrink-0" />
        {savedNote}
      </span>
    );
  }

  return (
    <div
      data-slot="status-line"
      className={cn('min-h-5 text-xs', className)}
      aria-live="polite"
      aria-atomic="true">
      {content}
    </div>
  );
};

export default StatusLine;
