/*
 * Background loop map + usage diagnostics.
 *
 * The loop map of the background work that runs without a chat message
 * (connection sync), plus the
 * recent-usage ledger + budget math. `view` lets a host panel (UsagePanel) mount just the ledger.
 */
import debug from 'debug';
import { RefreshCw } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';

import { listConnections as listComposioConnections } from '../../../../lib/composio/composioApi';
import type { ComposioConnection } from '../../../../lib/composio/types';
import { useT } from '../../../../lib/i18n/I18nContext';
import {
  creditsApi,
  type CreditTransaction,
  type TeamUsage,
} from '../../../../services/api/creditsApi';
import { Badge, Button, Card, StatusLine } from '../../../ui';
import {
  activeConnection,
  COMPOSIO_PERIODIC_TICK_MINUTES,
  formatCount,
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
  view = 'all',
  hideHeader = false,
}: {
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
  const composioPeriodicTicksPerWeek = Math.ceil(WEEK_MINUTES / COMPOSIO_PERIODIC_TICK_MINUTES);
  const composioConnectionScansPerWeek = composioPeriodicTicksPerWeek * activeConnections.length;
  const backgroundApiReadsPerWeek = composioConnectionScansPerWeek;
  const backgroundWakeupsPerWeek = composioPeriodicTicksPerWeek;
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

      {/* Loop map on top, the ledger + budget math below — stacked, so each
          card gets the full width instead of a cramped side column. */}
      <Card
        title={t('settings.ai.loopMap')}
        headerRight={
          <Button
            type="button"
            variant="secondary"
            size="sm"
            leadingIcon={<RefreshCw className="h-3.5 w-3.5" aria-hidden />}
            onClick={() => void refresh()}
            disabled={loading}>
            {t('common.refresh')}
          </Button>
        }>
        {loops.map(loop => (
          <div key={loop.name} className="grid gap-x-6 gap-y-2 px-4 py-3 md:grid-cols-[200px_1fr]">
            <div className="min-w-0 space-y-1.5">
              <div className="truncate text-sm font-medium text-content">{loop.name}</div>
              <div className="flex flex-wrap gap-1.5">
                <Badge variant={loop.enabled ? 'success' : 'neutral'}>
                  {loop.enabled ? t('settings.ai.on') : t('settings.ai.off')}
                </Badge>
                <Badge>{loop.cadence}</Badge>
              </div>
            </div>
            <div className="min-w-0 space-y-1 text-xs">
              <div className="text-content-secondary">{loop.work}</div>
              <div className="truncate font-mono text-content-muted">
                {t('settings.ai.routeLabel').replace('{route}', loop.route)}
              </div>
              <div className="text-content-faint">{loop.risk}</div>
            </div>
          </div>
        ))}
      </Card>

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
          composioConnectionScansPerWeek={composioConnectionScansPerWeek}
          estimatedRowsLeft={estimatedRowsLeft}
          estimatedRowsPerBudget={estimatedRowsPerBudget}
          projectedExhaustAt={projectedExhaustAt}
          projectedHoursLeft={projectedHoursLeft}
          scheduledCallsPerRemainingDollar={scheduledCallsPerRemainingDollar}
          activeConnectionsCount={activeConnections.length}
        />
      )}
    </div>
  );
};

export default BackgroundLoopControls;
