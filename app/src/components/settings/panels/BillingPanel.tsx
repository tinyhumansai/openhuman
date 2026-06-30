import { useT } from '../../../lib/i18n/I18nContext';
import { BILLING_DASHBOARD_URL } from '../../../utils/links';
import { openUrl } from '../../../utils/openUrl';
import Button from '../../ui/Button';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import SettingsPanel from '../layout/SettingsPanel';

const BillingPanel = () => {
  const { t } = useT();
  const { navigateBack } = useSettingsNavigation();

  return (
    <SettingsPanel>
      <div>
        <p className="text-xs font-semibold uppercase tracking-[0.2em] text-content-muted">
          {t('settings.billing.movedToWeb')}
        </p>
        <h1 className="mt-2 text-2xl font-semibold text-content">
          {t('settings.billing.openDashboard')}
        </h1>
        <p className="mt-2 text-sm leading-6 text-content-secondary">
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
    </SettingsPanel>
  );
};

export default BillingPanel;
