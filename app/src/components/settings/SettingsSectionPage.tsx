import type { ReactNode } from 'react';

import SettingsHeader from './components/SettingsHeader';
import SettingsMenuItem from './components/SettingsMenuItem';
import { useSettingsNavigation } from './hooks/useSettingsNavigation';

export interface SettingsSectionItem {
  id: string;
  title: string;
  description?: string;
  icon: ReactNode;
  /**
   * Settings sub-route to navigate to (under `/settings/`). Optional when an
   * explicit `onClick` is supplied — e.g. an item that links to a top-level
   * route outside the settings tree (the Alerts inbox at `/notifications`).
   */
  route?: string;
  /** Overrides the default `navigateToSettings(route)` navigation when set. */
  onClick?: () => void;
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
              onClick={item.onClick ?? (() => item.route && navigateToSettings(item.route))}
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
