import { type ReactNode, useEffect, useMemo, useState } from 'react';

import {
  type CostUsageCategoryStats,
  type CostUsageRecord,
  useCostDashboard,
  useCostUsageLog,
} from '../../hooks/useCostDashboard';
import { useT } from '../../lib/i18n/I18nContext';
import { SettingsStatusLine } from '../settings/controls';
import SettingsPanel from '../settings/layout/SettingsPanel';
import Button from '../ui/Button';
import BudgetSummary from './BudgetSummary';
import CostBarChart from './CostBarChart';
import DashboardSkeleton from './DashboardSkeleton';
import { formatCurrency, formatTokens, relativeTime } from './formatCurrency';
import ModelCostTable from './ModelCostTable';
import TokenUsageChart from './TokenUsageChart';

interface CostDashboardPanelProps {
  /** When true the panel is hosted inside another settings page (e.g. the
   *  Usage & Limits tabs) — skip the standalone SettingsHeader chrome. */
  embedded?: boolean;
}

const CostDashboardPanel = ({ embedded = false }: CostDashboardPanelProps) => {
  const { t } = useT();
  const { data, isLoading, isFetching, error, lastUpdated, refetch } = useCostDashboard();
  const {
    data: usageLog,
    isLoading: usageLogLoading,
    isFetching: usageLogFetching,
    error: usageLogError,
    lastUpdated: usageLogUpdated,
    refetch: refetchUsageLog,
  } = useCostUsageLog({ days: 30, limit: 250 });

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

  const body = (
    <>
      <div className="flex items-start justify-between gap-3">
        <p className="text-xs text-content-muted max-w-prose">
          {t('settings.costDashboard.subtitle')}
        </p>
        <div className="flex items-center gap-2 shrink-0">
          {(lastUpdated !== null || usageLogUpdated !== null) && (
            <span
              data-testid="cost-dashboard-updated"
              className="inline-flex items-center gap-1.5 text-[11px] text-content-muted">
              <span
                aria-hidden
                className={`inline-block h-1.5 w-1.5 rounded-full ${isFetching || usageLogFetching ? 'bg-ocean-500 animate-pulse' : 'bg-sage-500'}`}
              />
              {`${t('settings.costDashboard.updated')} ${relativeTime(Math.max(lastUpdated ?? 0, usageLogUpdated ?? 0), t)}`}
            </span>
          )}
          <Button
            type="button"
            variant="secondary"
            size="xs"
            data-testid="cost-dashboard-refresh"
            onClick={() => void Promise.all([refetch(), refetchUsageLog()])}
            disabled={isFetching || usageLogFetching}
            aria-label={t('settings.costDashboard.refresh')}
            leadingIcon={
              <RefreshIcon
                className={`h-3.5 w-3.5 ${isFetching || usageLogFetching ? 'animate-spin' : ''}`}
              />
            }>
            {t('settings.costDashboard.refresh')}
          </Button>
        </div>
      </div>

      {error && (
        <div role="alert" data-testid="cost-dashboard-error">
          <SettingsStatusLine saving={false} error={error} savingLabel="" />
        </div>
      )}
      {usageLogError && (
        <div role="alert" data-testid="cost-dashboard-usage-error">
          <SettingsStatusLine saving={false} error={usageLogError} savingLabel="" />
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
            className="rounded-2xl border border-line p-4 bg-surface/40">
            <header className="mb-2 flex items-baseline justify-between">
              <h2 className="text-sm font-semibold text-content">
                {t('settings.costDashboard.sevenDayCost')}
              </h2>
              <span className="text-[11px] text-content-muted">
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
            className="rounded-2xl border border-line p-4 bg-surface/40">
            <header className="mb-2 flex items-baseline justify-between">
              <h2 className="text-sm font-semibold text-content">
                {t('settings.costDashboard.sevenDayTokens')}
              </h2>
              <span className="text-[11px] text-content-muted">
                {t('settings.costDashboard.stackedNote')}
              </span>
            </header>
            <TokenUsageChart days={data.days} />
          </section>
          <section
            data-testid="cost-dashboard-model-table"
            className="rounded-2xl border border-line p-4 bg-surface/40">
            <header className="mb-2">
              <h2 className="text-sm font-semibold text-content">
                {t('settings.costDashboard.modelBreakdown')}
              </h2>
              <p className="text-[11px] text-content-muted">
                {t('settings.costDashboard.modelBreakdownHint')}
              </p>
            </header>
            <ModelCostTable models={data.by_model} currency={data.currency} />
          </section>
          <section
            data-testid="cost-dashboard-category-distribution"
            className="rounded-2xl border border-line p-4 bg-surface/40">
            <header className="mb-3">
              <h2 className="text-sm font-semibold text-content">
                {t('settings.costDashboard.categoryDistribution')}
              </h2>
              <p className="text-[11px] text-content-muted">
                {t('settings.costDashboard.categoryDistributionHint')}
              </p>
            </header>
            {usageLog ? (
              <CategoryDistribution
                categories={usageLog.by_category}
                currency={usageLog.currency}
              />
            ) : usageLogLoading ? (
              <div className="text-xs text-content-muted">
                {t('settings.costDashboard.loading')}
              </div>
            ) : null}
          </section>
          <section
            data-testid="cost-dashboard-usage-log"
            className="rounded-2xl border border-line p-4 bg-surface/40">
            <header className="mb-3 flex items-baseline justify-between gap-3">
              <div>
                <h2 className="text-sm font-semibold text-content">
                  {t('settings.costDashboard.usageLog')}
                </h2>
                <p className="text-[11px] text-content-muted">
                  {usageLog
                    ? t('settings.costDashboard.usageLogHint')
                        .replace('{days}', String(usageLog.days))
                        .replace('{limit}', String(usageLog.limit))
                    : t('settings.costDashboard.usageLogHint')
                        .replace('{days}', '30')
                        .replace('{limit}', '250')}
                </p>
              </div>
              {usageLog && (
                <span className="shrink-0 text-[11px] text-content-muted">
                  {t('settings.costDashboard.logTotal')
                    .replace('{requests}', String(usageLog.request_count))
                    .replace('{cost}', formatCurrency(usageLog.total_cost_usd, usageLog.currency))}
                </span>
              )}
            </header>
            {usageLog ? (
              <UsageLogTable records={usageLog.records} currency={usageLog.currency} />
            ) : usageLogLoading ? (
              <div className="text-xs text-content-muted">
                {t('settings.costDashboard.loading')}
              </div>
            ) : null}
          </section>
          {!hasAnyCost && (
            <div
              data-testid="cost-dashboard-empty"
              className="rounded-xl border border-dashed border-line-strong px-4 py-6 text-center">
              <div className="text-sm font-medium text-content-secondary">
                {t('settings.costDashboard.noData')}
              </div>
              <div className="text-[11px] text-content-muted mt-1">
                {t('settings.costDashboard.noDataHint')}
              </div>
            </div>
          )}
        </>
      )}
    </>
  );

  // Embedded inside the tabbed Usage & Limits page: the parent owns the header,
  // so render just the padded body.
  if (embedded)
    return (
      <div className="p-4 space-y-4" data-testid="cost-dashboard-panel">
        {body}
      </div>
    );

  return <SettingsPanel testId="cost-dashboard-panel">{body}</SettingsPanel>;
};

const CATEGORY_COLORS = [
  'bg-ocean-500',
  'bg-sage-500',
  'bg-amber-500',
  'bg-coral-500',
  'bg-neutral-500 dark:bg-neutral-400',
];

const CategoryDistribution = ({
  categories,
  currency,
}: {
  categories: CostUsageCategoryStats[];
  currency: string;
}) => {
  const { t } = useT();
  if (categories.length === 0) {
    return (
      <div className="text-xs text-content-muted italic py-2">
        {t('settings.costDashboard.noCategories')}
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <div aria-hidden className="flex h-3 w-full overflow-hidden rounded-full bg-surface-strong">
        {categories.map((category, index) => (
          <div
            key={category.category}
            className={CATEGORY_COLORS[index % CATEGORY_COLORS.length]}
            style={{ width: `${Math.max(0, Math.min(100, category.percent_of_total))}%` }}
          />
        ))}
      </div>
      <div className="grid gap-2 sm:grid-cols-2">
        {categories.map((category, index) => (
          <div key={category.category} className="rounded-lg border border-line px-3 py-2">
            <div className="flex items-center justify-between gap-3">
              <div className="flex min-w-0 items-center gap-2">
                <span
                  aria-hidden
                  className={`h-2 w-2 rounded-full ${CATEGORY_COLORS[index % CATEGORY_COLORS.length]}`}
                />
                <span className="truncate text-xs font-medium text-content">
                  {category.category}
                </span>
              </div>
              <span className="shrink-0 text-xs font-semibold tabular-nums text-content">
                {formatCurrency(category.cost_usd, currency)}
              </span>
            </div>
            <div className="mt-1 flex items-center justify-between gap-2 text-[11px] text-content-muted">
              <span>{`${category.percent_of_total.toFixed(1)}%`}</span>
              <span>
                {t('settings.costDashboard.categoryMeta')
                  .replace('{requests}', String(category.request_count))
                  .replace('{tokens}', formatTokens(category.total_tokens))}
              </span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
};

const UsageLogTable = ({ records, currency }: { records: CostUsageRecord[]; currency: string }) => {
  const { t } = useT();
  if (records.length === 0) {
    return (
      <div className="text-xs text-content-muted italic py-2">
        {t('settings.costDashboard.noUsageLog')}
      </div>
    );
  }

  return (
    <div className="overflow-x-auto -mx-1">
      <table className="w-full min-w-[760px] text-xs">
        <thead>
          <tr className="border-b border-line text-left text-[10px] uppercase tracking-wide text-content-muted">
            <Th>{t('settings.costDashboard.when')}</Th>
            <Th>{t('settings.costDashboard.category')}</Th>
            <Th>{t('settings.costDashboard.model')}</Th>
            <Th align="right">{t('settings.costDashboard.inputTokens')}</Th>
            <Th align="right">{t('settings.costDashboard.outputTokens')}</Th>
            <Th align="right">{t('settings.costDashboard.cost')}</Th>
            <Th>{t('settings.costDashboard.session')}</Th>
          </tr>
        </thead>
        <tbody>
          {records.map(record => (
            <tr
              key={record.id}
              className="border-b border-line-subtle transition-colors last:border-0 hover:bg-surface-muted/60 dark:border-line/60 dark:hover:bg-surface-muted/40">
              <Td>
                <div className="tabular-nums text-content-secondary">
                  {formatDateTime(record.timestamp)}
                </div>
              </Td>
              <Td>
                <span className="inline-flex rounded-full bg-surface-subtle px-2 py-0.5 text-[10px] font-medium text-content-secondary ring-1 ring-inset ring-neutral-200 dark:ring-neutral-700">
                  {record.category}
                </span>
              </Td>
              <Td>
                <div className="max-w-[16rem] truncate font-medium text-content">
                  {record.model}
                </div>
                <div className="text-[10px] text-content-muted">
                  {record.provider ?? t('settings.costDashboard.unknownProvider')}
                </div>
              </Td>
              <Td align="right">{formatTokens(record.input_tokens)}</Td>
              <Td align="right">{formatTokens(record.output_tokens)}</Td>
              <Td align="right">
                <span className="font-semibold tabular-nums text-content">
                  {formatCurrency(record.cost_usd, currency)}
                </span>
              </Td>
              <Td>
                <span className="font-mono text-[10px] text-content-muted">
                  {shortId(record.session_id)}
                </span>
              </Td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
};

interface CellProps {
  children: ReactNode;
  align?: 'left' | 'right';
}

const Th = ({ children, align = 'left' }: CellProps) => (
  <th className={`px-2 py-2 font-medium ${align === 'right' ? 'text-right' : 'text-left'}`}>
    {children}
  </th>
);

const Td = ({ children, align = 'left' }: CellProps) => (
  <td className={`px-2 py-2 align-middle ${align === 'right' ? 'text-right' : 'text-left'}`}>
    {children}
  </td>
);

function formatDateTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  }).format(date);
}

function shortId(value: string): string {
  return value.length > 8 ? value.slice(0, 8) : value;
}

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
