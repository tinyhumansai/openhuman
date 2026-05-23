import { ReactNode } from 'react';

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
  const titleColor = dangerous
    ? 'text-amber-600 dark:text-amber-300'
    : 'text-stone-900 dark:text-neutral-100';
  const iconColor = dangerous
    ? 'text-amber-600 dark:text-amber-300'
    : 'text-stone-900 dark:text-neutral-100';
  const borderColor = 'border-stone-200 dark:border-neutral-800';

  // Border classes for first/last items
  const borderClasses = isLast ? '' : `border-b ${borderColor}`;
  const roundedClasses = isFirst ? 'first:rounded-t-3xl' : isLast ? 'last:rounded-b-3xl' : '';

  const content = (
    <>
      <div className={`w-5 h-5 opacity-60 flex-shrink-0 mr-3 ${iconColor}`}>{icon}</div>
      <div className="flex-1">
        <div className={`font-medium text-sm mb-1 ${titleColor}`}>{title}</div>
        {description && <p className="opacity-70 text-xs">{description}</p>}
      </div>
      {rightElement && <div className="flex-shrink-0 ml-3">{rightElement}</div>}
    </>
  );

  if (onClick) {
    return (
      <button
        type="button"
        data-testid={testId}
        onClick={onClick}
        className={`w-full flex items-center justify-between py-3 px-4 bg-white dark:bg-neutral-900 text-stone-900 dark:text-neutral-100 ${borderClasses} hover:bg-stone-50 dark:hover:bg-neutral-800/60 dark:bg-neutral-800/60 dark:hover:bg-neutral-800/60 transition-all duration-200 text-left ${roundedClasses} focus:outline-none focus:ring-0 focus:border-inherit`}>
        {content}
      </button>
    );
  }

  return (
    <div
      data-testid={testId}
      className={`w-full flex items-center justify-between py-3 px-4 bg-white dark:bg-neutral-900 text-stone-900 dark:text-neutral-100 ${borderClasses} ${roundedClasses}`}>
      {content}
    </div>
  );
};

export default SettingsMenuItem;
