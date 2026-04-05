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
    <div className="z-10 relative">
      <SettingsHeader title={title} showBackButton={true} onBack={navigateBack} />

      <div>
        {description && <p className="mt-1 text-xs text-stone-500 px-5 pb-3">{description}</p>}

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
  );
};

export default SettingsSectionPage;
