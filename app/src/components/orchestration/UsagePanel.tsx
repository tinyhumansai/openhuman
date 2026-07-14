/**
 * UsagePanel — "stats on how much tokens, connections etc. have been spent".
 *
 * A read-only dashboard of the orchestration surface's footprint: credit
 * balance + cycle spend (billing), inference/integration call counts (team
 * usage), TokenJuice compaction savings, and the live connection count. Every
 * source is loaded independently (`Promise.allSettled`) so a single unavailable
 * backend (e.g. billing offline in a local build) degrades one tile to "—"
 * rather than blanking the page.
 */
import debugFactory from 'debug';
import { useEffect, useState } from 'react';

import { apiClient } from '../../agentworld/AgentWorldShell';
import { useT } from '../../lib/i18n/I18nContext';
import { type CreditBalance, creditsApi, type TeamUsage } from '../../services/api/creditsApi';
import { getTokenjuiceSavings, type SavingsStats } from '../../utils/tauriCommands/tokenjuice';
import { StatTile } from './primitives';

const debug = debugFactory('orchestration:usage');

function usd(value: number | undefined): string {
  if (value == null || !Number.isFinite(value)) return '—';
  return `$${value.toFixed(2)}`;
}

function count(value: number | undefined): string {
  if (value == null || !Number.isFinite(value)) return '—';
  return new Intl.NumberFormat().format(value);
}

interface UsageData {
  balance: CreditBalance | null;
  team: TeamUsage | null;
  savings: SavingsStats | null;
  connections: number | null;
}

export default function UsagePanel() {
  const { t } = useT();
  const [data, setData] = useState<UsageData | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    debug('usage load: entry');
    void Promise.allSettled([
      creditsApi.getBalance(),
      creditsApi.getTeamUsage(),
      getTokenjuiceSavings(),
      apiClient.orchestrationPairing.list(),
    ]).then(([balance, team, savings, pairing]) => {
      if (cancelled) return;
      debug(
        'usage load: exit balance=%s team=%s savings=%s pairing=%s',
        balance.status,
        team.status,
        savings.status,
        pairing.status
      );
      setData({
        balance: balance.status === 'fulfilled' ? balance.value : null,
        team: team.status === 'fulfilled' ? team.value : null,
        savings: savings.status === 'fulfilled' ? savings.value : null,
        connections:
          pairing.status === 'fulfilled'
            ? (pairing.value.stats?.contactCount ??
              pairing.value.contacts.contacts.filter(c => c.status === 'accepted').length)
            : null,
      });
      setLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  if (loading) {
    return (
      <p className="py-8 text-center text-sm text-content-muted" data-testid="orch-usage-loading">
        {t('tinyplaceOrchestration.loading')}
      </p>
    );
  }

  const balanceUsd =
    data?.balance != null
      ? data.balance.promotionBalanceUsd + data.balance.teamTopupUsd
      : undefined;
  const totals = data?.team?.insights.totals;

  return (
    <div className="space-y-3" data-testid="orch-usage-panel">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
        <StatTile
          label={t('orchPage.usage.connections')}
          value={count(data?.connections ?? undefined)}
          testId="orch-usage-connections"
        />
        <StatTile
          label={t('orchPage.usage.balance')}
          value={usd(balanceUsd)}
          hint={t('orchPage.usage.balanceHint')}
          testId="orch-usage-balance"
        />
        <StatTile
          label={t('orchPage.usage.cycleSpend')}
          value={usd(data?.team?.cycleSpentUsd)}
          hint={
            data?.team
              ? `${t('orchPage.usage.ofBudget')} ${usd(data.team.cycleBudgetUsd)}`
              : undefined
          }
          testId="orch-usage-cycle-spend"
        />
        <StatTile
          label={t('orchPage.usage.inferenceCalls')}
          value={count(totals?.inferenceCalls)}
          testId="orch-usage-inference-calls"
        />
        <StatTile
          label={t('orchPage.usage.integrationCalls')}
          value={count(totals?.integrationCalls)}
          testId="orch-usage-integration-calls"
        />
        <StatTile
          label={t('orchPage.usage.tokensSaved')}
          value={count(data?.savings?.total.tokensSaved)}
          hint={
            data?.savings
              ? `${usd(data.savings.total.costSavedUsd)} ${t('orchPage.usage.saved')}`
              : undefined
          }
          testId="orch-usage-tokens-saved"
        />
      </div>
      <p className="text-[11px] text-content-faint">{t('orchPage.usage.footnote')}</p>
    </div>
  );
}
