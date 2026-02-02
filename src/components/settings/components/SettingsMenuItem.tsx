import { ReactNode } from 'react';

interface SettingsMenuItemProps {
  icon: ReactNode;
  title: string;
  description?: string;
  onClick: () => void;
  dangerous?: boolean;
  isFirst?: boolean;
  isLast?: boolean;
}

const SettingsMenuItem = ({
  icon,
  title,
  description,
  onClick,
  dangerous = false,
  isFirst = false,
  isLast = false,
}: SettingsMenuItemProps) => {
  // Color variations for dangerous items (like logout/delete)
  const titleColor = dangerous ? 'text-amber-400' : 'text-white';
  const iconColor = dangerous ? 'text-amber-400' : 'text-white';
  const borderColor = 'border-stone-700'; // Use consistent border color for all items

  // Border classes for first/last items
  const borderClasses = isLast ? '' : `border-b ${borderColor}`;
  const roundedClasses = isFirst ? 'first:rounded-t-3xl' : isLast ? 'last:rounded-b-3xl' : '';

  return (
    <button
      onClick={onClick}
      className={`w-full flex items-center justify-between p-3 bg-black/50 ${borderClasses} hover:bg-stone-800/30 transition-all duration-200 text-left ${roundedClasses} focus:outline-none focus:ring-0 focus:border-inherit`}>
      <div className={`w-5 h-5 opacity-60 flex-shrink-0 mr-3 ${iconColor}`}>{icon}</div>
      <div className="flex-1">
        <div className={`font-medium text-sm mb-1 ${titleColor}`}>{title}</div>
        {description && <p className="opacity-70 text-xs">{description}</p>}
      </div>
    </button>
  );
};

export default SettingsMenuItem;
