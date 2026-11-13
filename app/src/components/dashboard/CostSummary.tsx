import { CalendarDays, type LucideIcon, TrendingUp, Wallet } from 'lucide-react';

import { useT } from '../../lib/i18n/I18nContext';
import { intlLocale } from '../../lib/i18n/intlLocale';
import { formatCurrency } from './formatCurrency';

interface CostSummaryProps {
  currency: string;
  periodTotalUsd: number;
  monthlyPaceUsd: number;
  monthToDateUsd: number;
}

/** Headline spend figures as three equal stat tiles. */
const CostSummary = ({
  currency,
  periodTotalUsd,
  monthlyPaceUsd,
  monthToDateUsd,
}: CostSummaryProps) => {
  const { t, locale } = useT();
  const numberLocale = intlLocale(locale);

  return (
    <section
      data-testid="cost-dashboard-summary"
      className="grid grid-cols-1 gap-3 sm:grid-cols-3"
      aria-label={t('settings.costDashboard.summaryAriaLabel')}>
      <StatTile
        testId="metric-total-spend"
        icon={Wallet}
        label={t('settings.costDashboard.totalSpend')}
        value={formatCurrency(periodTotalUsd, currency, { locale: numberLocale })}
        hint={t('settings.costDashboard.lastSevenDays')}
      />
      <StatTile
        testId="metric-month-to-date"
        icon={CalendarDays}
        label={t('settings.costDashboard.monthToDate')}
        value={formatCurrency(monthToDateUsd, currency, { locale: numberLocale })}
      />
      <StatTile
        testId="metric-monthly-pace"
        icon={TrendingUp}
        label={t('settings.costDashboard.monthlyPace')}
        value={formatCurrency(monthlyPaceUsd, currency, { locale: numberLocale })}
        hint={t('settings.costDashboard.monthlyPaceHint')}
      />
    </section>
  );
};

interface StatTileProps {
  icon: LucideIcon;
  label: string;
  value: string;
  hint?: string;
  testId: string;
}

const StatTile = ({ icon: Icon, label, value, hint, testId }: StatTileProps) => (
  <div data-testid={testId} className="rounded-xl border border-line bg-surface px-4 py-3.5">
    <div className="flex items-center gap-1.5 text-xs font-medium text-content-muted">
      <Icon className="h-3.5 w-3.5" aria-hidden />
      <span>{label}</span>
    </div>
    <div className="mt-1.5 text-2xl font-semibold tabular-nums text-content">{value}</div>
    {hint && <div className="mt-0.5 truncate text-xs text-content-faint">{hint}</div>}
  </div>
);

export default CostSummary;
