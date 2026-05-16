import createDebug from 'debug';
import { useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { BILLING_DASHBOARD_URL } from '../../../utils/links';
import { openUrl } from '../../../utils/openUrl';
import PageBackButton from '../components/PageBackButton';
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
    <div className="px-4 py-5 sm:px-6 lg:px-8">
      <div className="mx-auto max-w-3xl">
        <PageBackButton
          label={t('common.back')}
          onClick={navigateBack}
          trailingContent={
            breadcrumbs.length > 0 ? (
              <div className="flex flex-wrap items-center gap-2 text-xs text-stone-500">
                {breadcrumbs.map((crumb, index) => (
                  <button
                    key={`${crumb.label}-${index}`}
                    type="button"
                    onClick={crumb.onClick}
                    className="rounded-full border border-stone-200 bg-white px-3 py-1 font-medium text-stone-600 transition-colors hover:bg-stone-50">
                    {crumb.label}
                  </button>
                ))}
              </div>
            ) : null
          }
        />

        <div className="mt-6 rounded-3xl border border-stone-200 bg-white p-6 shadow-soft">
          <div className="max-w-xl space-y-4">
            <div>
              <p className="text-xs font-semibold uppercase tracking-[0.2em] text-stone-500">
                {t('settings.billing.movedToWeb')}
              </p>
              <h1 className="mt-2 text-2xl font-semibold text-stone-900">
                {t('settings.billing.openDashboard')}
              </h1>
              <p className="mt-2 text-sm leading-6 text-stone-600">
                {t('settings.billing.movedToWebDesc')}
              </p>
            </div>

            <div className="flex flex-wrap gap-3">
              <button
                type="button"
                onClick={() => {
                  void openUrl(BILLING_DASHBOARD_URL);
                }}
                className="inline-flex items-center rounded-full bg-primary-500 px-4 py-2 text-sm font-semibold text-white transition-colors hover:bg-primary-600">
                {t('settings.billing.openDashboard')}
              </button>
              <button
                type="button"
                onClick={navigateBack}
                className="inline-flex items-center rounded-full border border-stone-200 bg-white px-4 py-2 text-sm font-semibold text-stone-700 transition-colors hover:bg-stone-50">
                {t('settings.billing.backToSettings')}
              </button>
            </div>

            {status === 'opening' && (
              <p className="text-xs text-stone-500">{t('settings.billing.openingBrowser')}</p>
            )}
            {status === 'idle' && (
              <p className="text-xs text-stone-500">{t('settings.billing.browserNotOpen')}</p>
            )}
            {status === 'error' && (
              <p className="text-xs text-coral-600">{t('settings.billing.browserOpenFailed')}</p>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};

export default BillingPanel;
