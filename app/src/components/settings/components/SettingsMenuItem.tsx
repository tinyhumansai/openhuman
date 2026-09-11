import { ReactNode } from 'react';

import { cn } from '../../../lib/cn';
import Button from '../../ui/Button';

interface SettingsMenuItemProps {
  icon: ReactNode;
  title: string;
  description?: string;
  onClick?: () => void;
  testId?: string;
  dangerous?: boolean;
  isFirst?: boolean;
  isLast?: boolean;
  rightElement?: ReactNode;
}

const SettingsMenuItem = ({
  icon,
  title,
  description,
  onClick,
  testId,
  dangerous = false,
  isFirst = false,
  isLast = false,
  rightElement,
}: SettingsMenuItemProps) => {
  // Color variations for dangerous items (like logout/delete)
  const titleColor = dangerous ? 'text-amber-600 dark:text-amber-300' : 'text-content';
  const iconColor = dangerous ? 'text-amber-600 dark:text-amber-300' : 'text-content';

  // Border classes for first/last items
  const borderClasses = isLast ? '' : 'border-b border-line';
  const roundedClasses = isFirst ? 'first:rounded-t-3xl' : isLast ? 'last:rounded-b-3xl' : '';

  const content = (
    <>
      <div className={cn('w-5 h-5 opacity-60 shrink-0 mr-3', iconColor)}>{icon}</div>
      <div className="flex-1">
        <div className={cn('font-medium text-sm mb-1', titleColor)}>{title}</div>
        {description && <p className="opacity-70 text-xs">{description}</p>}
      </div>
      {rightElement && <div className="shrink-0 ml-3">{rightElement}</div>}
    </>
  );

  if (onClick) {
    return (
      <Button
        type="button"
        variant="tertiary"
        data-testid={testId}
        onClick={onClick}
        className={cn(
          'h-auto w-full flex items-center justify-between py-3 px-4 bg-surface-muted text-content',
          'hover:bg-surface-hover transition-all duration-200 text-left',
          borderClasses,
          roundedClasses
        )}>
        {content}
      </Button>
    );
  }

  return (
    <div
      data-testid={testId}
      className={cn(
        'w-full flex items-center justify-between py-3 px-4 bg-surface-muted text-content',
        borderClasses,
        roundedClasses
      )}>
      {content}
    </div>
  );
};

export default SettingsMenuItem;
