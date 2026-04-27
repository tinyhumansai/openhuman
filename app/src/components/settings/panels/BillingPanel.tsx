import createDebug from 'debug';
import { useEffect, useState } from 'react';

import PillTabBar from '../../../components/PillTabBar';
import { useCoreState } from '../../../providers/CoreStateProvider';
import { billingApi } from '../../../services/api/billingApi';
import {
  type AutoRechargeSettings,
  type CreditBalance,
  type CreditTransaction,
  creditsApi,
  type SavedCard,
} from '../../../services/api/creditsApi';
import type { CurrentPlanData, PlanTier } from '../../../types/api';
import { BILLING_DASHBOARD_URL } from '../../../utils/links';
import { openUrl } from '../../../utils/openUrl';
import PageBackButton from '../components/PageBackButton';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const log = createDebug('openhuman:billing-panel');

const BillingPanel = () => {
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
          label="Back"
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
                Billing moved to the web
              </p>
              <h1 className="mt-2 text-2xl font-semibold text-stone-900">Open billing dashboard</h1>
              <p className="mt-2 text-sm leading-6 text-stone-600">
                Subscription changes, payment methods, credits, and invoices are now managed at
                TinyHumans on the web.
              </p>
            </div>

            <div className="flex flex-wrap gap-3">
              <button
                type="button"
                onClick={() => {
                  void openUrl(BILLING_DASHBOARD_URL);
                }}
                className="inline-flex items-center rounded-full bg-primary-500 px-4 py-2 text-sm font-semibold text-white transition-colors hover:bg-primary-600">
                Open dashboard
              </button>
              <button
                type="button"
                onClick={navigateBack}
                className="inline-flex items-center rounded-full border border-stone-200 bg-white px-4 py-2 text-sm font-semibold text-stone-700 transition-colors hover:bg-stone-50">
                Back to settings
              </button>
            </div>

            {status === 'opening' && (
              <p className="text-xs text-stone-500">Opening your browser…</p>
            )}
            {status === 'idle' && (
              <p className="text-xs text-stone-500">
                If your browser did not open, use the button above.
              </p>
            )}
            {status === 'error' && (
              <p className="text-xs text-coral-600">
                The browser could not be opened automatically. Use the button above.
              </p>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};

export default BillingPanel;
