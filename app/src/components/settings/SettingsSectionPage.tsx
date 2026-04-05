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
}

const SettingsSectionPage = ({ title, description, items }: SettingsSectionPageProps) => {
  const { navigateBack, navigateToSettings } = useSettingsNavigation();

  return (
    <div className="overflow-hidden h-full flex flex-col z-10 relative">
      <SettingsHeader title={title} showBackButton={true} onBack={navigateBack} />

      <div className="flex-1 overflow-y-auto">
        <div className="p-4 space-y-4">
          {description && <p className="text-sm text-stone-500 px-1">{description}</p>}

          <div>
            {items.map((item, index) => (
              <SettingsMenuItem
                key={item.id}
                icon={item.icon}
                title={item.title}
                description={item.description}
                onClick={() => navigateToSettings(item.route)}
                isFirst={index === 0}
                isLast={index === items.length - 1}
              />
            ))}
          </div>
        </div>
      </div>
    </div>
  );
};

export default SettingsSectionPage;
