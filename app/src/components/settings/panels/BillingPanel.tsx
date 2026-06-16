import { useT } from '../../../lib/i18n/I18nContext';
import { BILLING_DASHBOARD_URL } from '../../../utils/links';
import { openUrl } from '../../../utils/openUrl';
import PanelPage from '../../layout/PanelPage';
import Button from '../../ui/Button';
import SettingsBackButton from '../components/SettingsBackButton';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const BillingPanel = () => {
  const { t } = useT();
  const { navigateBack } = useSettingsNavigation();

  return (
    <PanelPage
      className="z-10"
      contentClassName=""
      leading={<SettingsBackButton onBack={navigateBack} />}>
      <div className="p-4">
        <div className="max-w-xl space-y-4">
          <div>
            <p className="text-xs font-semibold uppercase tracking-[0.2em] text-neutral-500 dark:text-neutral-400">
              {t('settings.billing.movedToWeb')}
            </p>
            <h1 className="mt-2 text-2xl font-semibold text-neutral-800 dark:text-neutral-100">
              {t('settings.billing.openDashboard')}
            </h1>
            <p className="mt-2 text-sm leading-6 text-neutral-600 dark:text-neutral-300">
              {t('settings.billing.movedToWebDesc')}
            </p>
          </div>

          <div className="flex flex-wrap gap-3">
            <Button
              type="button"
              variant="primary"
              size="md"
              onClick={() => {
                void openUrl(BILLING_DASHBOARD_URL);
              }}>
              {t('settings.billing.openDashboard')}
            </Button>
            <Button type="button" variant="secondary" size="md" onClick={navigateBack}>
              {t('settings.billing.backToSettings')}
            </Button>
          </div>
        </div>
      </div>
    </PanelPage>
  );
};

export default BillingPanel;
