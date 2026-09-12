/*
 * Background loop map + usage diagnostics.
 *
 * The heartbeat planner, its calendar collector and the subconscious tick were
 * retired upstream along with the subconscious domain, so the planner controls
 * that used to occupy the left column are gone. What remains is the loop map of
 * the background work that still runs, plus the recent-usage ledger + budget
 * math. `view` lets a host panel (UsagePanel) mount just the ledger.
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { listConnections as listComposioConnections } from '../../../../lib/composio/composioApi';
import type { ComposioConnection } from '../../../../lib/composio/types';
import { useT } from '../../../../lib/i18n/I18nContext';
import {
  creditsApi,
  type CreditTransaction,
  type TeamUsage,
} from '../../../../services/api/creditsApi';
import Button from '../../../ui/Button';
import StatusLine from '../../../ui/StatusLine';
import type { RoutingMap } from './aiPanelTypes';
import {
  activeConnection,
  type BackgroundLoopProviderView,
  COMPOSIO_PERIODIC_TICK_MINUTES,
  describeProvider,
  formatCount,
  LEARNING_REBUILD_MINUTES,
  MEMORY_POLL_SECONDS,
  MEMORY_WORKERS,
  spendAmount,
  summarizeSpendByAction,
  summarizeSpendByHour,
  summarizeSpendSample,
  WEEK_MINUTES,
} from './backgroundLoopPrimitives';
import { UsageLedgerSection } from './UsageLedgerSection';

const log = debug('settings:background-loops');

type BackgroundLoopControlsView = 'all' | 'ledger';

export const BackgroundLoopControls = ({
  routing,
  cloudProviders,
  view = 'all',
  hideHeader = false,
}: {
  routing: RoutingMap;
  cloudProviders: BackgroundLoopProviderView[];
  view?: BackgroundLoopControlsView;
  hideHeader?: boolean;
}) => {
  const { t } = useT();
  const [usage, setUsage] = useState<TeamUsage | null>(null);
  const [transactions, setTransactions] = useState<CreditTransaction[]>([]);
  const [connections, setConnections] = useState<ComposioConnection[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>('');

  const refresh = useCallback(async () => {
    log('[settings:background-loops] refresh:start');
    setLoading(true);
    setError('');
    const [usageResult, transactionsResult, connectionsResult] = await Promise.allSettled([
      creditsApi.getTeamUsage(),
      creditsApi.getTransactions(200, 0),
      listComposioConnections(),
    ]);

    log(
      '[settings:background-loops] refresh:settled usage=%s transactions=%s connections=%s',
      usageResult.status,
      transactionsResult.status,
      connectionsResult.status
    );

    if (usageResult.status === 'fulfilled') {
      setUsage(usageResult.value);
    }

    if (transactionsResult.status === 'fulfilled') {
      const rows = transactionsResult.value.transactions ?? [];
      log('[settings:background-loops] refresh:transactions count=%d', rows.length);
      setTransactions(rows);
    }

    if (connectionsResult.status === 'fulfilled') {
      const rows = connectionsResult.value.connections ?? [];
      log('[settings:background-loops] refresh:connections count=%d', rows.length);
      setConnections(rows);
    }
    setLoading(false);
    log('[settings:background-loops] refresh:done');
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void refresh();
  }, [refresh]);

  const spendSample = summarizeSpendSample(transactions);
  const spendRows = spendSample.rows;
  const actionSummary = summarizeSpendByAction(transactions);
  const hourSummary = summarizeSpendByHour(transactions);
  const latestSpend = spendRows[0] ?? null;
  const activeConnections = connections.filter(activeConnection);
  // The heartbeat planner, its calendar collector and the subconscious tick
  // went with the subconscious domain, so none of them contributes scheduled
  // wakeups or background API reads any more. Kept as named zeros so the
  // ledger arithmetic below still reads as "everything that runs in the
  // background" instead of silently dropping the terms.
  const heartbeatTicksPerWeek = 0;
  const calendarPlannerCallsPerWeek = 0;
  const composioPeriodicTicksPerWeek = Math.ceil(WEEK_MINUTES / COMPOSIO_PERIODIC_TICK_MINUTES);
  const learningTicksPerWeek = Math.ceil(WEEK_MINUTES / LEARNING_REBUILD_MINUTES);
  const memoryPollsPerWeek = Math.ceil((WEEK_MINUTES * 60 * MEMORY_WORKERS) / MEMORY_POLL_SECONDS);
  const composioConnectionScansPerWeek = composioPeriodicTicksPerWeek * activeConnections.length;
  const backgroundApiReadsPerWeek = calendarPlannerCallsPerWeek + composioConnectionScansPerWeek;
  const backgroundWakeupsPerWeek =
    heartbeatTicksPerWeek +
    composioPeriodicTicksPerWeek +
    learningTicksPerWeek +
    memoryPollsPerWeek;
  const scheduledCallsPerRemainingDollar =
    usage && usage.remainingUsd > 0 ? backgroundApiReadsPerWeek / usage.remainingUsd : null;
  const estimatedRowsLeft =
    usage && spendSample.avgRowUsd > 0
      ? Math.floor(usage.remainingUsd / spendSample.avgRowUsd)
      : null;
  const estimatedRowsPerBudget =
    usage && spendSample.avgRowUsd > 0
      ? Math.floor(usage.cycleBudgetUsd / spendSample.avgRowUsd)
      : null;
  const projectedHoursLeft =
    usage && spendSample.spendPerHour > 0 ? usage.remainingUsd / spendSample.spendPerHour : null;
  const projectionAnchorMs = latestSpend ? new Date(latestSpend.createdAt).getTime() : Number.NaN;
  const projectedExhaustAt =
    projectedHoursLeft !== null && Number.isFinite(projectionAnchorMs)
      ? new Date(projectionAnchorMs + projectedHoursLeft * 3_600_000).toLocaleString([], {
          month: 'short',
          day: 'numeric',
          hour: 'numeric',
          minute: '2-digit',
        })
      : 'n/a';

  const loops = [
    {
      name: t('settings.ai.loops.memoryTreeWorkers.name'),
      enabled: true,
      cadence: t('settings.ai.loops.cadence.queue'),
      route: describeProvider(routing.memory, cloudProviders),
      work: t('settings.ai.loops.memoryTreeWorkers.work'),
      risk: t('settings.ai.loops.memoryTreeWorkers.risk')
        .replace('{workers}', String(MEMORY_WORKERS))
        .replace('{seconds}', String(MEMORY_POLL_SECONDS)),
    },
    {
      name: t('settings.ai.loops.reflectionRebuild.name'),
      enabled: true,
      cadence: t('settings.ai.loops.cadence.thirtyMin'),
      route: describeProvider(routing.learning, cloudProviders),
      work: t('settings.ai.loops.reflectionRebuild.work'),
      risk: t('settings.ai.loops.reflectionRebuild.risk').replace(
        '{count}',
        formatCount(learningTicksPerWeek)
      ),
    },
    {
      name: t('settings.ai.loops.composioSync.name'),
      enabled: true,
      cadence: t('settings.ai.loops.cadence.twentyMin'),
      route: t('settings.ai.loops.composioSync.route'),
      work: t('settings.ai.loops.composioSync.work'),
      risk: t('settings.ai.loops.composioSync.risk')
        .replace('{count}', formatCount(composioPeriodicTicksPerWeek))
        .replace('{active}', String(activeConnections.length)),
    },
  ];

  const showLedger = view === 'all' || view === 'ledger';
  const gridCols =
    view === 'all' ? 'md:grid-cols-[minmax(0,1fr)_minmax(260px,0.8fr)]' : 'grid-cols-1';

  return (
    <div className="space-y-4">
      {!hideHeader && (
        <div className="border-b border-line pb-2">
          <h2 className="text-base font-semibold text-content">
            {t('settings.ai.backgroundLoops')}
          </h2>
          <p className="mt-0.5 text-xs text-content-muted">
            {t('settings.ai.backgroundLoopsDesc')}
          </p>
        </div>
      )}

      {error && <StatusLine saving={false} error={error} savedNote={null} savingLabel="" />}

      <section className={`grid gap-3 ${gridCols}`}>
        <div className="overflow-hidden rounded-lg border border-line bg-surface-muted">
          <div className="flex items-center justify-between gap-3 border-b border-line px-3 py-2">
            <span className="text-xs font-semibold uppercase tracking-wide text-content-faint">
              {t('settings.ai.loopMap')}
            </span>
            <Button
              type="button"
              variant="secondary"
              size="xs"
              onClick={() => void refresh()}
              disabled={loading}>
              {t('common.refresh')}
            </Button>
          </div>
          <div className="divide-y divide-line">
            {loops.map(loop => (
              <div key={loop.name} className="grid gap-2 px-3 py-3 md:grid-cols-[150px_1fr]">
                <div className="min-w-0">
                  <div className="truncate text-sm font-medium text-content">{loop.name}</div>
                  <div className="mt-0.5 flex flex-wrap gap-1 text-[11px] text-content-muted">
                    <span>{loop.enabled ? t('settings.ai.on') : t('settings.ai.off')}</span>
                    <span>{loop.cadence}</span>
                  </div>
                </div>
                <div className="min-w-0 text-xs text-content-secondary">
                  <div>{loop.work}</div>
                  <div className="mt-1 font-mono text-[11px] text-content-muted">
                    {t('settings.ai.routeLabel').replace('{route}', loop.route)}
                  </div>
                  <div className="mt-1 text-content-muted">{loop.risk}</div>
                </div>
              </div>
            ))}
          </div>
        </div>

        {showLedger && (
          <UsageLedgerSection
            t={t}
            loading={loading}
            onRefresh={() => void refresh()}
            usage={usage}
            spendRows={spendRows}
            spendAvgRowUsd={spendSample.avgRowUsd}
            spendSampleHours={spendSample.sampleHours}
            spendPerHour={spendSample.spendPerHour}
            rowsPerHour={spendSample.rowsPerHour}
            actionSummary={actionSummary}
            hourSummary={hourSummary}
            latestSpend={latestSpend}
            formatSpendAmount={spendAmount}
            backgroundApiReadsPerWeek={backgroundApiReadsPerWeek}
            backgroundWakeupsPerWeek={backgroundWakeupsPerWeek}
            calendarPlannerCallsPerWeek={calendarPlannerCallsPerWeek}
            composioConnectionScansPerWeek={composioConnectionScansPerWeek}
            memoryPollsPerWeek={memoryPollsPerWeek}
            estimatedRowsLeft={estimatedRowsLeft}
            estimatedRowsPerBudget={estimatedRowsPerBudget}
            projectedExhaustAt={projectedExhaustAt}
            projectedHoursLeft={projectedHoursLeft}
            scheduledCallsPerRemainingDollar={scheduledCallsPerRemainingDollar}
            activeConnectionsCount={activeConnections.length}
          />
        )}
      </section>
    </div>
  );
};

export default BackgroundLoopControls;
