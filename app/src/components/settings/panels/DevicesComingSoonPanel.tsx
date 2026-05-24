import { useT } from '../../../lib/i18n/I18nContext';
import EmptyStateCard from '../../EmptyStateCard';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const DevicesComingSoonPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  return (
    <div className="z-10 relative">
      <div className="px-5 pt-5 pb-3">
        <SettingsHeader
          title={t('settings.devices')}
          showBackButton={breadcrumbs.length > 0}
          onBack={navigateBack}
          breadcrumbs={breadcrumbs}
        />
      </div>

      <div className="px-5 pb-5">
        <EmptyStateCard
          className="shadow-soft"
          icon={
            <svg
              className="h-7 w-7 text-primary-500"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={1.5}
              aria-hidden="true">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M12 18h.01M8 21h8a2 2 0 002-2V5a2 2 0 00-2-2H8a2 2 0 00-2 2v14a2 2 0 002 2z"
              />
            </svg>
          }
          title="Devices"
          description="Device pairing is coming soon. This page will be the home for pairing iPhones and managing connected devices."
          footer={
            <span className="mt-4 inline-flex items-center rounded-full bg-primary-50 dark:bg-primary-500/10 px-3 py-1 text-xs font-medium text-primary-600 dark:text-primary-400">
              Coming Soon
            </span>
          }
        />
      </div>
    </div>
  );
};

export default DevicesComingSoonPanel;
