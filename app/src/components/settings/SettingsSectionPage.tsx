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

      {/* Mirror the SettingsHome layout: padded container, items in a single
          rounded-border card, and the optional footer in its own matching card
          so section pages and the home list look identical. */}
      <div className="px-4 pb-5">
        {description && (
          <p className="mb-3 px-1 text-xs text-stone-500 dark:text-neutral-400">{description}</p>
        )}

        <div className="rounded-3xl overflow-hidden border border-stone-200 dark:border-neutral-800">
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

        {footer && (
          <>
            {/* Divider + card, mirroring how SettingsHome separates its
                trailing groups (e.g. the destructive logout/clear card). */}
            <div className="mx-1 mt-6 mb-2 border-t border-stone-200 dark:border-neutral-800" />
            <div className="rounded-3xl overflow-hidden border border-stone-200 dark:border-neutral-800">
              {footer}
            </div>
          </>
        )}
      </div>
    </div>
  );
};

export default SettingsSectionPage;
