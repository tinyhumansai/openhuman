import type { ReactNode } from 'react';

import type { BudgetStatus } from '../../hooks/useCostDashboard';
import { useT } from '../../lib/i18n/I18nContext';
import { formatCurrency } from './formatCurrency';

export interface BudgetSummaryProps {
  currency: string;
  periodTotalUsd: number;
  monthlyPaceUsd: number;
  budgetLimitMonthlyUsd: number;
  monthToDateUsd: number;
  utilization: number;
  status: BudgetStatus;
}

const STATUS_BG: Record<BudgetStatus, string> = {
  normal: 'bg-sage-500',
  warning: 'bg-amber-500',
  exceeded: 'bg-coral-500',
};

const STATUS_TEXT: Record<BudgetStatus, string> = {
  normal: 'text-sage-600 dark:text-sage-300',
  warning: 'text-amber-600 dark:text-amber-300',
  exceeded: 'text-coral-600 dark:text-coral-300',
};

const STATUS_LABEL_KEY: Record<BudgetStatus, string> = {
  normal: 'settings.costDashboard.budgetNormal',
  warning: 'settings.costDashboard.budgetWarning',
  exceeded: 'settings.costDashboard.budgetExceeded',
};

const BudgetSummary = ({
  currency,
  periodTotalUsd,
  monthlyPaceUsd,
  budgetLimitMonthlyUsd,
  monthToDateUsd,
  utilization,
  status,
}: BudgetSummaryProps) => {
  const { t } = useT();
  const utilizationPct = Math.round(utilization * 100);
  const utilizationClamped = Math.min(100, Math.max(0, utilizationPct));

  return (
    <section
      data-testid="cost-dashboard-summary"
      className="grid grid-cols-1 md:grid-cols-3 gap-3"
      aria-label={t('settings.costDashboard.summaryAriaLabel')}>
      {/* Hero tile: 7-day total + status badge + progress bar */}
      <div
        data-testid="metric-total-spend"
        className="md:col-span-2 rounded-2xl border border-stone-200 dark:border-neutral-800 bg-gradient-to-br from-ocean-50 to-white dark:from-neutral-900 dark:to-neutral-950 p-5 flex flex-col gap-3 shadow-soft">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2 text-xs uppercase tracking-wide text-stone-500 dark:text-neutral-400">
            <WalletIcon className="h-4 w-4" />
            <span>{t('settings.costDashboard.totalSpend')}</span>
          </div>
          <span
            data-testid="budget-status-badge"
            className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-[11px] font-medium text-white ${STATUS_BG[status]}`}>
            <span
              aria-hidden
              className="inline-block h-1.5 w-1.5 rounded-full bg-white/80 animate-pulse"
            />
            {t(STATUS_LABEL_KEY[status])}
          </span>
        </div>
        <div className="flex items-baseline gap-3">
          <span className="text-3xl md:text-4xl font-semibold tabular-nums text-stone-900 dark:text-neutral-50">
            {formatCurrency(periodTotalUsd, currency)}
          </span>
          <span className="text-xs text-stone-500 dark:text-neutral-400">
            {t('settings.costDashboard.lastSevenDays')}
          </span>
        </div>
        <div className="flex flex-col gap-1">
          <div className="flex items-center justify-between text-[11px] text-stone-500 dark:text-neutral-400">
            <span>
              {budgetLimitMonthlyUsd > 0
                ? `${formatCurrency(monthToDateUsd, currency)} ${t('settings.costDashboard.utilizationOf')} ${formatCurrency(budgetLimitMonthlyUsd, currency)} ${t('settings.costDashboard.thisMonth')}`
                : `${formatCurrency(monthToDateUsd, currency)} ${t('settings.costDashboard.thisMonth')}`}
            </span>
            <span className={`font-medium tabular-nums ${STATUS_TEXT[status]}`}>
              {`${utilizationPct}%`}
            </span>
          </div>
          <div
            aria-hidden
            className="h-2 w-full rounded-full bg-stone-200 dark:bg-neutral-800 overflow-hidden">
            <div
              data-testid="utilization-fill"
              className={`h-full rounded-full transition-all duration-300 ${STATUS_BG[status]}`}
              style={{ width: `${utilizationClamped}%` }}
            />
          </div>
        </div>
      </div>

      <div className="grid grid-cols-2 md:grid-cols-1 gap-3">
        <SmallMetric
          icon={<TrendingUpIcon className="h-4 w-4" />}
          label={t('settings.costDashboard.monthlyPace')}
          value={formatCurrency(monthlyPaceUsd, currency)}
          hint={t('settings.costDashboard.monthlyPaceHint')}
          testId="metric-monthly-pace"
        />
        <SmallMetric
          icon={<TargetIcon className="h-4 w-4" />}
          label={t('settings.costDashboard.budgetLimit')}
          value={
            budgetLimitMonthlyUsd > 0
              ? formatCurrency(budgetLimitMonthlyUsd, currency)
              : t('settings.costDashboard.noBudget')
          }
          hint={t('settings.costDashboard.budgetLimitHint')}
          testId="metric-budget-limit"
        />
      </div>
    </section>
  );
};

interface SmallMetricProps {
  icon: ReactNode;
  label: string;
  value: string;
  hint: string;
  testId: string;
}

const SmallMetric = ({ icon, label, value, hint, testId }: SmallMetricProps) => (
  <div
    data-testid={testId}
    title={hint}
    className="rounded-2xl border border-stone-200 dark:border-neutral-800 p-3 flex flex-col gap-1 hover:border-ocean-300 dark:hover:border-ocean-700 transition-colors">
    <div className="flex items-center gap-2 text-[11px] uppercase tracking-wide text-stone-500 dark:text-neutral-400">
      {icon}
      <span>{label}</span>
    </div>
    <span className="text-lg font-semibold tabular-nums text-stone-800 dark:text-neutral-100">
      {value}
    </span>
  </div>
);

interface IconProps {
  className?: string;
}

const WalletIcon = ({ className }: IconProps) => (
  <svg
    className={className}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden>
    <path d="M21 12V7H5a2 2 0 0 1 0-4h14v4" />
    <path d="M3 5v14a2 2 0 0 0 2 2h16v-5" />
    <circle cx="17" cy="14" r="1.5" />
  </svg>
);

const TrendingUpIcon = ({ className }: IconProps) => (
  <svg
    className={className}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden>
    <polyline points="23 6 13.5 15.5 8.5 10.5 1 18" />
    <polyline points="17 6 23 6 23 12" />
  </svg>
);

const TargetIcon = ({ className }: IconProps) => (
  <svg
    className={className}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden>
    <circle cx="12" cy="12" r="10" />
    <circle cx="12" cy="12" r="6" />
    <circle cx="12" cy="12" r="2" />
  </svg>
);

export default BudgetSummary;
