import { type ReactNode } from 'react';

import Button from '../../ui/Button';

export interface SettingsListItemProps {
  label: string;
  badge?: ReactNode;
  onRemove?: () => void;
  removeLabel: string;
  mono?: boolean;
  'data-testid'?: string;
}

const SettingsListItem = ({
  label,
  badge,
  onRemove,
  removeLabel,
  mono = false,
  'data-testid': testId,
}: SettingsListItemProps) => {
  const labelClass = ['text-xs text-content truncate', mono ? 'font-mono' : '']
    .filter(Boolean)
    .join(' ');

  return (
    <li className="flex items-center justify-between gap-3 px-4 py-2.5" data-testid={testId}>
      <div className="flex items-center gap-2 min-w-0 flex-1">
        <span className={labelClass}>{label}</span>
        {badge && <span className="flex-shrink-0">{badge}</span>}
      </div>
      {onRemove && (
        <Button
          type="button"
          variant="tertiary"
          size="xs"
          onClick={onRemove}
          aria-label={removeLabel}
          className="text-coral-500 dark:text-coral-400 hover:text-coral-600 dark:hover:text-coral-300 flex-shrink-0">
          {removeLabel}
        </Button>
      )}
    </li>
  );
};

export default SettingsListItem;
