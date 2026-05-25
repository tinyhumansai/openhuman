import type { ReactNode } from 'react';

import SettingsHeader from './components/SettingsHeader';
import SettingsMenuItem from './components/SettingsMenuItem';
import { useSettingsNavigation } from './hooks/useSettingsNavigation';

export interface SettingsSectionItem {
  id: string;
  title: string;
  description?: string;
  icon: ReactNode;
  route: string;
}

interface SettingsSectionPageProps {
  title: string;
  description?: string;
  items: SettingsSectionItem[];
  /** Optional content rendered below the items list (e.g. destructive actions). */
  footer?: ReactNode;
}

const SettingsSectionPage = ({ title, description, items, footer }: SettingsSectionPageProps) => {
  const { navigateBack, navigateToSettings, breadcrumbs } = useSettingsNavigation();

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title={title}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div>
        {description && (
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400 px-5 pb-3">
            {description}
          </p>
        )}

        <div>
          {items.map((item, index) => (
            <SettingsMenuItem
              key={item.id}
              icon={item.icon}
              title={item.title}
              description={item.description}
              onClick={() => navigateToSettings(item.route)}
              testId={`settings-nav-${item.id}`}
              isFirst={index === 0}
              isLast={index === items.length - 1}
            />
          ))}
        </div>

        {footer}
      </div>
    </div>
  );
};

export default SettingsSectionPage;
