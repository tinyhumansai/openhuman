/*
 * Usage ledger + budget-math section — the right column of
 * BackgroundLoopControls. Pure presentational component: all figures are
 * precomputed by the parent hook.
 */
import { RefreshCw } from 'lucide-react';

import type { CreditTransaction, TeamUsage } from '../../../../services/api/creditsApi';
import { Button, Card, EmptyState } from '../../../ui';
import {
  formatCount,
  formatDateTime,
  formatUsd,
  FormulaRow,
  MetricTile,
} from './backgroundLoopPrimitives';

export const UsageLedgerSection = ({
  t,
  loading,
  onRefresh,
  usage,
  spendRows,
  spendAvgRowUsd,
  spendSampleHours,
  spendPerHour,
  rowsPerHour,
  actionSummary,
  hourSummary,
  latestSpend,
  formatSpendAmount,
  backgroundApiReadsPerWeek,
  backgroundWakeupsPerWeek,
  composioConnectionScansPerWeek,
  estimatedRowsLeft,
  estimatedRowsPerBudget,
  projectedExhaustAt,
  projectedHoursLeft,
  scheduledCallsPerRemainingDollar,
  activeConnectionsCount,
}: {
  t: (key: string, fallback?: string) => string;
  loading: boolean;
  onRefresh: () => void;
  usage: TeamUsage | null;
  spendRows: CreditTransaction[];
  spendAvgRowUsd: number;
  spendSampleHours: number;
  spendPerHour: number;
  rowsPerHour: number;
  actionSummary: Array<[string, number, number]>;
  hourSummary: Array<[string, number]>;
  latestSpend: CreditTransaction | null;
  formatSpendAmount: (tx: CreditTransaction) => number;
  backgroundApiReadsPerWeek: number;
  backgroundWakeupsPerWeek: number;
  composioConnectionScansPerWeek: number;
  estimatedRowsLeft: number | null;
  estimatedRowsPerBudget: number | null;
  projectedExhaustAt: string;
  projectedHoursLeft: number | null;
  scheduledCallsPerRemainingDollar: number | null;
  activeConnectionsCount: number;
}) => (
  <div className="space-y-4">
    <Card
      title={t('settings.ai.recentUsageLedger')}
      description={t('settings.ai.recentUsageLedgerDesc')}
      headerRight={
        <Button
          type="button"
          variant="secondary"
          size="sm"
          leadingIcon={<RefreshCw className="h-3.5 w-3.5" aria-hidden />}
          onClick={onRefresh}
          disabled={loading}>
          {t('common.reload')}
        </Button>
      }>
      <div className="grid grid-cols-2 gap-2 p-4 lg:grid-cols-3">
        <MetricTile
          label={t('settings.ai.weekBudget')}
          value={usage ? formatUsd(usage.cycleBudgetUsd) : t('common.notAvailable')}
          detail={t('settings.ai.resetsAt').replace('{time}', formatDateTime(usage?.cycleEndsAt))}
        />
        <MetricTile
          label={t('settings.ai.cycleRemaining')}
          value={usage ? formatUsd(usage.remainingUsd) : t('common.notAvailable')}
          detail={
            usage
              ? t('settings.ai.usedAmount').replace('{amount}', formatUsd(usage.cycleSpentUsd))
              : undefined
          }
        />
        <MetricTile
          label={t('settings.ai.cycleTotalSpend')}
          value={usage ? formatUsd(usage.insights.totals.totalUsd) : t('common.notAvailable')}
          detail={
            usage
              ? t('settings.ai.inferenceIntegrationsBreakdown')
                  .replace('{inference}', formatUsd(usage.insights.totals.inferenceUsd))
                  .replace('{integrations}', formatUsd(usage.insights.totals.integrationsUsd))
              : undefined
          }
        />
        <MetricTile
          label={t('settings.ai.avgSpendRow')}
          value={spendAvgRowUsd > 0 ? formatUsd(spendAvgRowUsd) : t('common.notAvailable')}
          detail={t('settings.ai.recentSpendRowsCount').replace(
            '{count}',
            String(spendRows.length)
          )}
        />
        <MetricTile
          label={t('settings.ai.backgroundApiReads')}
          value={t('settings.ai.perWeek').replace(
            '{count}',
            formatCount(backgroundApiReadsPerWeek)
          )}
          detail={t('settings.ai.connectionSyncBreakdown').replace(
            '{sync}',
            formatCount(composioConnectionScansPerWeek)
          )}
        />
        <MetricTile
          label={t('settings.ai.backgroundWakeups')}
          value={t('settings.ai.perWeek').replace('{count}', formatCount(backgroundWakeupsPerWeek))}
        />
      </div>
      {latestSpend && (
        <div className="px-4 py-3 text-xs text-content-secondary">
          {t('settings.ai.latestSpend')
            .replace('{amount}', formatUsd(formatSpendAmount(latestSpend)))
            .replace('{time}', new Date(latestSpend.createdAt).toLocaleString())
            .replace('{action}', latestSpend.action)}
        </div>
      )}
    </Card>

    <div className="grid gap-4 xl:grid-cols-2">
      <Card title={t('settings.ai.budgetMath')}>
        <FormulaRow
          label={t('settings.ai.rowsLeft')}
          value={
            estimatedRowsLeft !== null ? formatCount(estimatedRowsLeft) : t('common.notAvailable')
          }
          detail={
            estimatedRowsLeft !== null
              ? t('settings.ai.rowsLeftFormula')
                  .replace('{remaining}', formatUsd(usage?.remainingUsd ?? 0))
                  .replace('{avgRow}', formatUsd(spendAvgRowUsd))
              : t('settings.ai.needSpendRowsToEstimate')
          }
        />
        <FormulaRow
          label={t('settings.ai.rowsPerFullWeekBudget')}
          value={
            estimatedRowsPerBudget !== null
              ? formatCount(estimatedRowsPerBudget)
              : t('common.notAvailable')
          }
          detail={
            estimatedRowsPerBudget !== null
              ? t('settings.ai.rowsPerBudgetFormula')
                  .replace('{budget}', formatUsd(usage?.cycleBudgetUsd ?? 0))
                  .replace('{avgRow}', formatUsd(spendAvgRowUsd))
              : t('settings.ai.needSpendRowsToEstimate')
          }
        />
        <FormulaRow
          label={t('settings.ai.sampleBurnRate')}
          value={
            spendPerHour > 0
              ? t('settings.ai.perHour').replace('{amount}', formatUsd(spendPerHour))
              : t('common.notAvailable')
          }
          detail={
            spendSampleHours > 0
              ? t('settings.ai.burnRateSampleDetail')
                  .replace('{rows}', formatCount(rowsPerHour))
                  .replace('{hours}', spendSampleHours.toFixed(1))
              : t('settings.ai.needTimestampsForBurnRate')
          }
        />
        <FormulaRow
          label={t('settings.ai.projectedEmpty')}
          value={projectedExhaustAt}
          detail={
            projectedHoursLeft !== null
              ? t('settings.ai.projectedEmptyDetail').replace(
                  '{hours}',
                  projectedHoursLeft.toFixed(1)
                )
              : t('settings.ai.noProjectionWithoutSpend')
          }
        />
        <FormulaRow
          label={t('settings.ai.apiReadsPerDollarRemaining')}
          value={
            scheduledCallsPerRemainingDollar !== null
              ? t('settings.ai.readsPerDollar').replace(
                  '{count}',
                  formatCount(scheduledCallsPerRemainingDollar)
                )
              : t('common.notAvailable')
          }
          detail={
            usage
              ? t('settings.ai.apiReadsFormula')
                  .replace('{reads}', formatCount(backgroundApiReadsPerWeek))
                  .replace('{remaining}', formatUsd(usage.remainingUsd))
              : t('settings.ai.needUsageToEstimate')
          }
        />
      </Card>

      <Card title={t('settings.ai.loopCallBudget')}>
        <FormulaRow
          label={t('settings.ai.composioSyncScans')}
          value={t('settings.ai.perWeek').replace(
            '{count}',
            formatCount(composioConnectionScansPerWeek)
          )}
          detail={t('settings.ai.composioSyncScansDetail').replace(
            '{count}',
            String(activeConnectionsCount)
          )}
        />
        <FormulaRow
          label={t('settings.ai.totalBackgroundApiReadBudget')}
          value={t('settings.ai.perWeek').replace(
            '{count}',
            formatCount(backgroundApiReadsPerWeek)
          )}
          detail={t('settings.ai.totalApiReadBudgetDetail')}
        />
      </Card>
    </div>

    <div className="grid gap-4 md:grid-cols-2">
      <Card title={t('settings.ai.topActions')}>
        {actionSummary.length > 0 ? (
          actionSummary.map(([action, count, total]) => (
            <div
              key={action}
              className="flex items-center justify-between gap-3 px-4 py-2.5 text-xs text-content-secondary">
              <span className="truncate font-mono">{action}</span>
              <span className="shrink-0 tabular-nums text-content-muted">
                {count} / {formatUsd(total)}
              </span>
            </div>
          ))
        ) : (
          <EmptyState label={t('settings.ai.noSpendRows')} />
        )}
      </Card>

      <Card title={t('settings.ai.topHours')}>
        {hourSummary.length > 0 ? (
          hourSummary.map(([hour, total]) => (
            <div
              key={hour}
              className="flex items-center justify-between gap-3 px-4 py-2.5 text-xs text-content-secondary">
              <span>{hour}</span>
              <span className="font-mono tabular-nums text-content-muted">{formatUsd(total)}</span>
            </div>
          ))
        ) : (
          <EmptyState label={t('settings.ai.noHourlySpend')} />
        )}
      </Card>
    </div>
  </div>
);

export default UsageLedgerSection;
