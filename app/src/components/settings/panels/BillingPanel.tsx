import createDebug from 'debug';
import { useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { BILLING_DASHBOARD_URL } from '../../../utils/links';
import { openUrl } from '../../../utils/openUrl';
import Button from '../../ui/Button';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const log = createDebug('openhuman:billing-panel');

const BillingPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const [status, setStatus] = useState<'opening' | 'idle' | 'error'>('opening');

  useEffect(() => {
    let cancelled = false;

    const openDashboard = async () => {
      log('[redirect] opening billing dashboard url=%s', BILLING_DASHBOARD_URL);
      try {
        await openUrl(BILLING_DASHBOARD_URL);
        if (!cancelled) {
          setStatus('idle');
        }
      } catch (error) {
        log('[redirect] failed to open billing dashboard: %O', error);
        if (!cancelled) {
          setStatus('error');
        }
      }
    };

    void openDashboard();

    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title={t('nav.avatarMenu.billing')}
        showBackButton
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

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

          {status === 'opening' && (
            <p className="text-xs text-neutral-500 dark:text-neutral-400">
              {t('settings.billing.openingBrowser')}
            </p>
          )}
          {status === 'idle' && (
            <p className="text-xs text-neutral-500 dark:text-neutral-400">
              {t('settings.billing.browserNotOpen')}
            </p>
          )}
          {status === 'error' && (
            <p className="text-xs text-coral-600 dark:text-coral-300">
              {t('settings.billing.browserOpenFailed')}
            </p>
          )}
        </div>
      </div>
    </div>
  );
};

export default BillingPanel;
