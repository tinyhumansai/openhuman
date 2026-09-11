import { type ReactNode } from 'react';

import { cn } from '../../lib/cn';
import Button from './Button';

export interface ListRowProps {
  label: string;
  badge?: ReactNode;
  onRemove?: () => void;
  removeLabel: string;
  /**
   * Render the remove control as an icon button instead of a text button.
   * `removeLabel` stays the accessible name (and the tooltip), so nothing is
   * lost from the a11y tree — this only changes what is painted. Needed by
   * dense rows (a canvas drawer, a chip list) where a full "Remove" word does
   * not fit next to a truncating label.
   */
  removeIcon?: ReactNode;
  /** `data-testid` for the remove control itself, not the row. */
  removeTestId?: string;
  mono?: boolean;
  className?: string;
  'data-testid'?: string;
}

/** A removable row inside a `<ul>` — allow-list entries, installed items, tags. */
const ListRow = ({
  label,
  badge,
  onRemove,
  removeLabel,
  removeIcon,
  removeTestId,
  mono = false,
  className,
  'data-testid': testId,
}: ListRowProps) => (
  <li
    data-slot="list-row"
    className={cn('flex items-center justify-between gap-3 px-4 py-2.5', className)}
    data-testid={testId}>
    <div className="flex min-w-0 flex-1 items-center gap-2">
      <span className={cn('truncate text-xs text-content', mono && 'font-mono')}>{label}</span>
      {badge && <span className="shrink-0">{badge}</span>}
    </div>
    {onRemove && (
      <Button
        type="button"
        variant="tertiary"
        size="xs"
        iconOnly={removeIcon != null}
        onClick={onRemove}
        aria-label={removeLabel}
        title={removeIcon != null ? removeLabel : undefined}
        data-testid={removeTestId}
        className="shrink-0 text-coral-500 hover:text-coral-600 dark:text-coral-400 dark:hover:text-coral-300">
        {removeIcon ?? removeLabel}
      </Button>
    )}
  </li>
);

export default ListRow;
