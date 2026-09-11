import createDebug from 'debug';
import { useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { billingApi } from '../../../services/api/billingApi';
import { creditsApi, type TeamUsage } from '../../../services/api/creditsApi';
import type { BillingSummaryData } from '../../../types/api';
import { BILLING_DASHBOARD_URL } from '../../../utils/links';
import { openUrl } from '../../../utils/openUrl';
import Button from '../../ui/Button';
import { SettingsStatusLine } from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import SettingsPanel from '../layout/SettingsPanel';
import InferenceBudget from './billing/InferenceBudget';

const log = createDebug('openhuman:billing:panel');
const formatUsd = (amount: unknown, unavailable: string): string =>
  typeof amount === 'number' && Number.isFinite(amount) && amount >= 0
    ? `$${amount.toFixed(2)}`
    : unavailable;

const BillingPanel = () => {
  const { t } = useT();
  const { navigateBack } = useSettingsNavigation();
  const [summary, setSummary] = useState<BillingSummaryData | null>(null);
  const [teamUsage, setTeamUsage] = useState<TeamUsage | null>(null);
  const [summaryLoading, setSummaryLoading] = useState(true);
  const [usageLoading, setUsageLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    const recordError = (failure: unknown) => {
      const message = failure instanceof Error ? failure.message : String(failure);
      setError(current => current ?? message);
    };

    log('loading billing summary and cycle usage');
    void billingApi
      .getSummary()
      .then(value => {
        if (!cancelled) setSummary(value);
      })
      .catch(failure => {
        log('summary load failed error=%s', String(failure));
        if (!cancelled) recordError(failure);
      })
      .finally(() => {
        if (!cancelled) setSummaryLoading(false);
      });
    void creditsApi
      .getTeamUsage()
      .then(value => {
        if (!cancelled) setTeamUsage(value);
      })
      .catch(failure => {
        log('usage load failed error=%s', String(failure));
        if (!cancelled) recordError(failure);
      })
      .finally(() => {
        if (!cancelled) setUsageLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, []);

  // Only /teams/me/usage includes the remaining subscription-cycle allowance.
  // The summary's totalUsd is wallet-only (promotion + top-up), so it is not a
  // safe fallback for the account's true available balance.
  const availableUsd = teamUsage?.remainingUsd;
  const unavailable = t('settings.billing.inferenceBudget.notAvailable');
  const topUpUrl = summary?.links?.topUpUrl || `${BILLING_DASHBOARD_URL}?tab=billing`;
  const manageUrl = summary?.links?.manageUrl || BILLING_DASHBOARD_URL;

  return (
    <SettingsPanel>
      <SettingsStatusLine
        saving={summaryLoading || usageLoading}
        error={error}
        savingLabel={t('common.loading')}
      />

      <section className="rounded-2xl border border-line bg-surface p-4 space-y-4">
        <div className="space-y-1">
          <h2 className="text-base font-semibold text-content">
            {t('settings.billing.movedToWeb')}
          </h2>
          <p className="text-sm text-content-muted">{t('settings.billing.movedToWebDesc')}</p>
        </div>

        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
          <SummaryTile
            label={t('settings.billing.subscription.currentPlan')}
            value={
              summary?.plan?.plan ??
              teamUsage?.plan?.plan ??
              (summaryLoading || usageLoading ? '...' : unavailable)
            }
          />
          <SummaryTile
            label={t('settings.billing.payAsYouGo.available')}
            value={
              availableUsd === undefined
                ? usageLoading
                  ? '...'
                  : unavailable
                : formatUsd(availableUsd, unavailable)
            }
          />
          <SummaryTile
            label={t('settings.billing.payAsYouGo.promotionalCredits')}
            value={
              summary
                ? formatUsd(summary.credits?.promotionBalanceUsd, unavailable)
                : summaryLoading
                  ? '...'
                  : unavailable
            }
          />
          <SummaryTile
            label={t('settings.billing.payAsYouGo.topUpBalance')}
            value={
              summary
                ? formatUsd(summary.credits?.teamTopupUsd, unavailable)
                : summaryLoading
                  ? '...'
                  : unavailable
            }
          />
        </div>
      </section>

      <InferenceBudget teamUsage={teamUsage} isLoadingCredits={usageLoading} />

      <div className="flex flex-wrap gap-3">
        <Button type="button" variant="primary" size="md" onClick={() => void openUrl(topUpUrl)}>
          {t('settings.billing.payAsYouGo.topUpCredits')}
        </Button>
        <Button type="button" variant="secondary" size="md" onClick={() => void openUrl(manageUrl)}>
          {t('settings.billing.openDashboard')}
        </Button>
        <Button type="button" variant="tertiary" size="md" onClick={navigateBack}>
          {t('settings.billing.backToSettings')}
        </Button>
      </div>
    </SettingsPanel>
  );
};

const SummaryTile = ({ label, value }: { label: string; value: string }) => (
  <div className="rounded-xl border border-line bg-surface-muted px-3 py-3">
    <div className="text-[11px] text-content-muted">{label}</div>
    <div className="mt-1 text-lg font-semibold text-content">{value}</div>
  </div>
);

export default BillingPanel;
