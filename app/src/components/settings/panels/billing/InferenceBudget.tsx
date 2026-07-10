import { useT } from '../../../../lib/i18n/I18nContext';
import type {
  TeamUsage,
  TeamUsageDailyPoint,
  TeamUsageIntegrationRow,
  TeamUsageModelRow,
} from '../../../../services/api/creditsApi';

interface InferenceBudgetProps {
  teamUsage: TeamUsage | null;
  isLoadingCredits: boolean;
}

const fmtUsd = (n: number): string => `$${(n ?? 0).toFixed(2)}`;

const formatCycleEnds = (iso: string, notAvailable: string): string => {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return notAvailable;
  // Use UTC so a UTC-midnight cycle end doesn't shift a day in the user's TZ.
  return date.toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
    timeZone: 'UTC',
  });
};

const InferenceBudget = ({ teamUsage, isLoadingCredits }: InferenceBudgetProps) => {
  const { t } = useT();

  return (
    <div className="rounded-2xl border border-line bg-surface p-3 space-y-3">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-content">
          {t('settings.billing.inferenceBudget.title')}
        </h3>
        {isLoadingCredits && (
          <span className="text-[10px] text-content-muted">{t('common.loading')}</span>
        )}
        {teamUsage && !isLoadingCredits && (
          <span className="text-xs text-content-faint">
            {teamUsage.cycleBudgetUsd > 0
              ? t('settings.billing.inferenceBudget.remainingSummary')
                  .replace('{remaining}', fmtUsd(teamUsage.remainingUsd))
                  .replace('{budget}', fmtUsd(teamUsage.cycleBudgetUsd))
              : t('settings.billing.inferenceBudget.noRecurringPlanBudget')}
          </span>
        )}
      </div>

      {teamUsage ? (
        <>
          {teamUsage.cycleBudgetUsd > 0 ? (
            <>
              <div className="h-1.5 bg-surface-strong rounded-full overflow-hidden">
                <div
                  className={`h-full rounded-full transition-all duration-300 ${
                    teamUsage.remainingUsd <= 0
                      ? 'bg-coral-500'
                      : teamUsage.remainingUsd / teamUsage.cycleBudgetUsd < 0.2
                        ? 'bg-amber-500'
                        : 'bg-primary-500'
                  }`}
                  style={{
                    width: `${Math.max(
                      0,
                      Math.min(100, (teamUsage.remainingUsd / teamUsage.cycleBudgetUsd) * 100)
                    )}%`,
                  }}
                />
              </div>
              <div className="flex items-center justify-between">
                <span className="text-[11px] text-content-muted">
                  {t('settings.billing.inferenceBudget.spentThisCycle').replace(
                    '{amount}',
                    fmtUsd(teamUsage.cycleSpentUsd)
                  )}
                </span>
                <span className="text-[11px] text-content-muted">
                  {t('settings.billing.inferenceBudget.cycleEndsOn').replace(
                    '{date}',
                    formatCycleEnds(
                      teamUsage.cycleEndsAt,
                      t('settings.billing.inferenceBudget.notAvailable')
                    )
                  )}
                </span>
              </div>
              {teamUsage.remainingUsd <= 0 && (
                <p className="text-[11px] text-coral-400">
                  {t('settings.billing.inferenceBudget.exhaustedDesc')}
                </p>
              )}
            </>
          ) : (
            <div className="rounded-xl border border-line bg-surface-muted px-3 py-2.5">
              <p className="text-[11px] text-content-secondary">
                {t('settings.billing.inferenceBudget.noRecurringWeeklyDesc')}
              </p>
            </div>
          )}

          {teamUsage.plan.discountVsPayAsYouGoPercent > 0 && (
            <div className="rounded-xl border border-primary-100 bg-primary-50 px-3 py-2 text-[11px] text-primary-700">
              <span className="font-semibold">{teamUsage.plan.name}:</span>{' '}
              {t('settings.billing.inferenceBudget.discountVsPayg').replace(
                '{pct}',
                String(teamUsage.plan.discountVsPayAsYouGoPercent)
              )}
            </div>
          )}

          <UsageBreakdown
            totalUsd={teamUsage.insights.totals.totalUsd}
            inferenceUsd={teamUsage.insights.totals.inferenceUsd}
            integrationsUsd={teamUsage.insights.totals.integrationsUsd}
            inferenceCalls={teamUsage.insights.totals.inferenceCalls}
            integrationCalls={teamUsage.insights.totals.integrationCalls}
          />

          <DailyChart points={teamUsage.insights.dailySeries} />

          <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
            <TopModels rows={teamUsage.insights.topModels} />
            <TopIntegrations rows={teamUsage.insights.topIntegrations} />
          </div>
        </>
      ) : isLoadingCredits ? (
        <div className="h-1.5 w-full rounded-full bg-surface-strong animate-pulse" />
      ) : (
        <p className="text-xs text-content-muted">
          {t('settings.billing.inferenceBudget.unableToLoad')}
        </p>
      )}
    </div>
  );
};

const UsageBreakdown = ({
  totalUsd,
  inferenceUsd,
  integrationsUsd,
  inferenceCalls,
  integrationCalls,
}: {
  totalUsd: number;
  inferenceUsd: number;
  integrationsUsd: number;
  inferenceCalls: number;
  integrationCalls: number;
}) => {
  const { t } = useT();

  return (
    <div className="rounded-xl border border-line bg-surface-muted px-3 py-2">
      <div className="flex items-center justify-between mb-1.5">
        <span className="text-[10px] font-semibold uppercase tracking-wide text-content-faint">
          {t('settings.billing.inferenceBudget.cycleSpend')}
        </span>
        <span className="text-[11px] text-content-secondary">
          {t('settings.billing.inferenceBudget.totalAmount').replace('{amount}', fmtUsd(totalUsd))}
        </span>
      </div>
      <div className="grid grid-cols-2 gap-2 text-[11px]">
        <div>
          <div className="text-content-muted">
            {t('settings.billing.inferenceBudget.inference')}
          </div>
          <div className="text-content font-medium">{fmtUsd(inferenceUsd)}</div>
          <div className="text-content-faint">
            {t('settings.billing.inferenceBudget.calls').replace(
              '{count}',
              inferenceCalls.toLocaleString()
            )}
          </div>
        </div>
        <div>
          <div className="text-content-muted">
            {t('settings.billing.inferenceBudget.integrations')}
          </div>
          <div className="text-content font-medium">{fmtUsd(integrationsUsd)}</div>
          <div className="text-content-faint">
            {t('settings.billing.inferenceBudget.calls').replace(
              '{count}',
              integrationCalls.toLocaleString()
            )}
          </div>
        </div>
      </div>
    </div>
  );
};

const DailyChart = ({ points }: { points: TeamUsageDailyPoint[] }) => {
  const { t } = useT();

  if (points.length === 0) {
    return null;
  }
  const max = points.reduce((m, p) => Math.max(m, p.totalUsd), 0) || 1;
  return (
    <div className="rounded-xl border border-line bg-surface px-3 py-2">
      <div className="text-[10px] font-semibold uppercase tracking-wide text-content-faint mb-2">
        {t('settings.billing.inferenceBudget.dailySpend')}
      </div>
      <div className="flex items-end gap-1 h-16">
        {points.map(p => {
          const inferenceHeight = (p.inferenceUsd / max) * 100;
          const integrationsHeight = (p.integrationsUsd / max) * 100;
          return (
            <div
              key={p.date}
              className="flex-1 h-full flex flex-col-reverse"
              title={t('settings.billing.inferenceBudget.dailySpendPoint')
                .replace('{date}', p.date)
                .replace('{amount}', fmtUsd(p.totalUsd))}>
              <div className="bg-primary-400" style={{ height: `${inferenceHeight}%` }} />
              <div className="bg-amber-400" style={{ height: `${integrationsHeight}%` }} />
            </div>
          );
        })}
      </div>
      <div className="flex items-center gap-3 mt-1.5 text-[10px] text-content-muted">
        <span className="flex items-center gap-1">
          <span className="w-2 h-2 bg-primary-400 inline-block" />{' '}
          {t('settings.billing.inferenceBudget.inference')}
        </span>
        <span className="flex items-center gap-1">
          <span className="w-2 h-2 bg-amber-400 inline-block" />{' '}
          {t('settings.billing.inferenceBudget.integrations')}
        </span>
      </div>
    </div>
  );
};

const TopModels = ({ rows }: { rows: TeamUsageModelRow[] }) => {
  const { t } = useT();

  return (
    <div className="rounded-xl border border-line bg-surface px-3 py-2">
      <div className="text-[10px] font-semibold uppercase tracking-wide text-content-faint mb-1.5">
        {t('settings.billing.inferenceBudget.topModels')}
      </div>
      {rows.length === 0 ? (
        <p className="text-[11px] text-content-muted">
          {t('settings.billing.inferenceBudget.noInferenceUsage')}
        </p>
      ) : (
        <ul className="space-y-0.5">
          {rows.map((r, i) => (
            <li
              key={`${r.provider}::${r.model}::${i}`}
              className="flex items-center justify-between text-[11px]">
              <span className="text-content-secondary truncate mr-2">{r.model || r.provider}</span>
              <span className="text-content-muted flex-shrink-0">
                {fmtUsd(r.spentUsd)} · {r.calls.toLocaleString()}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};

const TopIntegrations = ({ rows }: { rows: TeamUsageIntegrationRow[] }) => {
  const { t } = useT();

  return (
    <div className="rounded-xl border border-line bg-surface px-3 py-2">
      <div className="text-[10px] font-semibold uppercase tracking-wide text-content-faint mb-1.5">
        {t('settings.billing.inferenceBudget.topIntegrations')}
      </div>
      {rows.length === 0 ? (
        <p className="text-[11px] text-content-muted">
          {t('settings.billing.inferenceBudget.noIntegrationUsage')}
        </p>
      ) : (
        <ul className="space-y-0.5">
          {rows.map((r, i) => (
            <li
              key={`${r.provider}::${r.action}::${i}`}
              className="flex items-center justify-between text-[11px]">
              <span className="text-content-secondary truncate mr-2">
                {r.provider}
                {r.action ? ` · ${r.action}` : ''}
              </span>
              <span className="text-content-muted flex-shrink-0">
                {fmtUsd(r.spentUsd)} · {r.calls.toLocaleString()}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};

export default InferenceBudget;
