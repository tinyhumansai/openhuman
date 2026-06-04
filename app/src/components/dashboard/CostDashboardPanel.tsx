import { useEffect, useMemo, useState } from 'react';

import { useCostDashboard } from '../../hooks/useCostDashboard';
import { useT } from '../../lib/i18n/I18nContext';
import SettingsHeader from '../settings/components/SettingsHeader';
import { useSettingsNavigation } from '../settings/hooks/useSettingsNavigation';
import BudgetSummary from './BudgetSummary';
import CostBarChart from './CostBarChart';
import DashboardSkeleton from './DashboardSkeleton';
import { relativeTime } from './formatCurrency';
import ModelCostTable from './ModelCostTable';
import TokenUsageChart from './TokenUsageChart';

const CostDashboardPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const { data, isLoading, isFetching, error, lastUpdated, refetch } = useCostDashboard();

  const hasAnyCost = useMemo(
    () => (data ? data.days.some(day => day.cost_usd > 0) : false),
    [data]
  );

  // Tick once a second so the "Updated Ns ago" pill stays fresh without
  // re-rendering the entire chart pipeline.
  const [, setTick] = useState(0);
  useEffect(() => {
    const id = window.setInterval(() => setTick(n => n + 1), 1000);
    return () => window.clearInterval(id);
  }, []);

  return (
    <div className="z-10 relative" data-testid="cost-dashboard-panel">
      <SettingsHeader
        title={t('settings.costDashboard.title')}
        showBackButton
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />
      <div className="p-4 space-y-4">
        <div className="flex items-start justify-between gap-3">
          <p className="text-xs text-stone-500 dark:text-neutral-400 max-w-prose">
            {t('settings.costDashboard.subtitle')}
          </p>
          <div className="flex items-center gap-2 shrink-0">
            {lastUpdated !== null && (
              <span
                data-testid="cost-dashboard-updated"
                className="inline-flex items-center gap-1.5 text-[11px] text-stone-500 dark:text-neutral-400">
                <span
                  aria-hidden
                  className={`inline-block h-1.5 w-1.5 rounded-full ${isFetching ? 'bg-ocean-500 animate-pulse' : 'bg-sage-500'}`}
                />
                {`${t('settings.costDashboard.updated')} ${relativeTime(lastUpdated, t)}`}
              </span>
            )}
            <button
              type="button"
              data-testid="cost-dashboard-refresh"
              onClick={() => void refetch()}
              disabled={isFetching}
              aria-label={t('settings.costDashboard.refresh')}
              className="inline-flex items-center gap-1 rounded-md border border-stone-200 dark:border-neutral-800 px-2 py-1 text-[11px] text-stone-700 dark:text-neutral-200 hover:bg-stone-100 dark:hover:bg-neutral-800 disabled:opacity-50 transition-colors">
              <RefreshIcon className={`h-3.5 w-3.5 ${isFetching ? 'animate-spin' : ''}`} />
              <span>{t('settings.costDashboard.refresh')}</span>
            </button>
          </div>
        </div>

        {error && (
          <div
            role="alert"
            className="rounded-md border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-xs text-coral-700 dark:text-coral-300"
            data-testid="cost-dashboard-error">
            {error}
          </div>
        )}
        {data && !data.enabled && (
          <div
            className="rounded-md border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-300"
            data-testid="cost-dashboard-disabled">
            {t('settings.costDashboard.disabledHint')}
          </div>
        )}

        {!data && isLoading && <DashboardSkeleton />}

        {data && (
          <>
            <BudgetSummary
              currency={data.currency}
              periodTotalUsd={data.period_total_usd}
              monthlyPaceUsd={data.monthly_pace_usd}
              budgetLimitMonthlyUsd={data.budget_limit_monthly_usd}
              monthToDateUsd={data.month_to_date_usd}
              utilization={data.budget_utilization}
              status={data.budget_status}
            />
            <section
              data-testid="cost-dashboard-cost-chart"
              className="rounded-2xl border border-stone-200 dark:border-neutral-800 p-4 bg-white/40 dark:bg-neutral-900/40">
              <header className="mb-2 flex items-baseline justify-between">
                <h2 className="text-sm font-semibold text-stone-800 dark:text-neutral-100">
                  {t('settings.costDashboard.sevenDayCost')}
                </h2>
                <span className="text-[11px] text-stone-500 dark:text-neutral-400">
                  {t('settings.costDashboard.utcNote')}
                </span>
              </header>
              <CostBarChart
                days={data.days}
                currency={data.currency}
                budgetLimitMonthlyUsd={data.budget_limit_monthly_usd}
                warnThreshold={data.warn_threshold}
                alertThreshold={data.alert_threshold}
              />
            </section>
            <section
              data-testid="cost-dashboard-token-chart"
              className="rounded-2xl border border-stone-200 dark:border-neutral-800 p-4 bg-white/40 dark:bg-neutral-900/40">
              <header className="mb-2 flex items-baseline justify-between">
                <h2 className="text-sm font-semibold text-stone-800 dark:text-neutral-100">
                  {t('settings.costDashboard.sevenDayTokens')}
                </h2>
                <span className="text-[11px] text-stone-500 dark:text-neutral-400">
                  {t('settings.costDashboard.stackedNote')}
                </span>
              </header>
              <TokenUsageChart days={data.days} />
            </section>
            <section
              data-testid="cost-dashboard-model-table"
              className="rounded-2xl border border-stone-200 dark:border-neutral-800 p-4 bg-white/40 dark:bg-neutral-900/40">
              <header className="mb-2">
                <h2 className="text-sm font-semibold text-stone-800 dark:text-neutral-100">
                  {t('settings.costDashboard.modelBreakdown')}
                </h2>
                <p className="text-[11px] text-stone-500 dark:text-neutral-400">
                  {t('settings.costDashboard.modelBreakdownHint')}
                </p>
              </header>
              <ModelCostTable models={data.by_model} currency={data.currency} />
            </section>
            {!hasAnyCost && (
              <div
                data-testid="cost-dashboard-empty"
                className="rounded-xl border border-dashed border-stone-300 dark:border-neutral-700 px-4 py-6 text-center">
                <div className="text-sm font-medium text-stone-700 dark:text-neutral-200">
                  {t('settings.costDashboard.noData')}
                </div>
                <div className="text-[11px] text-stone-500 dark:text-neutral-400 mt-1">
                  {t('settings.costDashboard.noDataHint')}
                </div>
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
};

interface IconProps {
  className?: string;
}

const RefreshIcon = ({ className }: IconProps) => (
  <svg
    className={className}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden>
    <polyline points="23 4 23 10 17 10" />
    <polyline points="1 20 1 14 7 14" />
    <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10" />
    <path d="M20.49 15a9 9 0 0 1-14.85 3.36L1 14" />
  </svg>
);

export default CostDashboardPanel;
