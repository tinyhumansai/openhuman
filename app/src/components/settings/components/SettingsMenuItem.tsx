import { ReactNode } from 'react';

interface SettingsMenuItemProps {
  icon: ReactNode;
  title: string;
  description?: string;
  onClick?: () => void;
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
  dangerous = false,
  isFirst = false,
  isLast = false,
  rightElement,
}: SettingsMenuItemProps) => {
  // Color variations for dangerous items (like logout/delete)
  const titleColor = dangerous ? 'text-amber-600' : 'text-stone-900';
  const iconColor = dangerous ? 'text-amber-600' : 'text-stone-900';
  const borderColor = 'border-stone-200'; // Use consistent border color for all items

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
        onClick={onClick}
        className={`w-full flex items-center justify-between py-3 px-4 bg-white ${borderClasses} hover:bg-stone-50 transition-all duration-200 text-left ${roundedClasses} focus:outline-none focus:ring-0 focus:border-inherit`}>
        {content}
      </button>
    );
  }

  return (
    <div
      className={`w-full flex items-center justify-between py-3 px-4 bg-white ${borderClasses} ${roundedClasses}`}>
      {content}
    </div>
  );
};

export default SettingsMenuItem;
