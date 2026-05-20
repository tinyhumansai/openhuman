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

const formatCycleEnds = (iso: string): string => {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return 'n/a';
  // Use UTC so a UTC-midnight cycle end doesn't shift a day in the user's TZ.
  return date.toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
    timeZone: 'UTC',
  });
};

const InferenceBudget = ({ teamUsage, isLoadingCredits }: InferenceBudgetProps) => (
  <div className="rounded-2xl border border-stone-200 bg-white p-3 space-y-3">
    <div className="flex items-center justify-between">
      <h3 className="text-sm font-semibold text-stone-900">Inference Budget</h3>
      {isLoadingCredits && <span className="text-[10px] text-stone-500">Loading…</span>}
      {teamUsage && !isLoadingCredits && (
        <span className="text-xs text-stone-400">
          {teamUsage.cycleBudgetUsd > 0
            ? `${fmtUsd(teamUsage.remainingUsd)} / ${fmtUsd(teamUsage.cycleBudgetUsd)} remaining`
            : 'No recurring plan budget'}
        </span>
      )}
    </div>

    {teamUsage ? (
      <>
        {teamUsage.cycleBudgetUsd > 0 ? (
          <>
            <div className="h-1.5 bg-stone-200 rounded-full overflow-hidden">
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
              <span className="text-[11px] text-stone-500">
                Spent {fmtUsd(teamUsage.cycleSpentUsd)} this cycle
              </span>
              <span className="text-[11px] text-stone-500">
                Cycle ends {formatCycleEnds(teamUsage.cycleEndsAt)}
              </span>
            </div>
            {teamUsage.remainingUsd <= 0 && (
              <p className="text-[11px] text-coral-400">
                Included subscription usage is exhausted. Top up credits to keep using AI without
                waiting for the next cycle.
              </p>
            )}
          </>
        ) : (
          <div className="rounded-xl border border-stone-200 bg-stone-50 px-3 py-2.5">
            <p className="text-[11px] text-stone-600">
              Your current plan does not include a recurring weekly inference budget. Usage is paid
              from available credits instead.
            </p>
          </div>
        )}

        {teamUsage.plan.discountVsPayAsYouGoPercent > 0 && (
          <div className="rounded-xl border border-primary-100 bg-primary-50 px-3 py-2 text-[11px] text-primary-700">
            <span className="font-semibold">{teamUsage.plan.name}:</span>{' '}
            {teamUsage.plan.discountVsPayAsYouGoPercent}% cheaper per call than pay-as-you-go.
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
      <div className="h-1.5 w-full rounded-full bg-stone-200 animate-pulse" />
    ) : (
      <p className="text-xs text-stone-500">Unable to load usage data</p>
    )}
  </div>
);

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
}) => (
  <div className="rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
    <div className="flex items-center justify-between mb-1.5">
      <span className="text-[10px] font-semibold uppercase tracking-wide text-stone-400">
        Cycle spend
      </span>
      <span className="text-[11px] text-stone-600">{fmtUsd(totalUsd)} total</span>
    </div>
    <div className="grid grid-cols-2 gap-2 text-[11px]">
      <div>
        <div className="text-stone-500">Inference</div>
        <div className="text-stone-900 font-medium">{fmtUsd(inferenceUsd)}</div>
        <div className="text-stone-400">{inferenceCalls.toLocaleString()} calls</div>
      </div>
      <div>
        <div className="text-stone-500">Integrations</div>
        <div className="text-stone-900 font-medium">{fmtUsd(integrationsUsd)}</div>
        <div className="text-stone-400">{integrationCalls.toLocaleString()} calls</div>
      </div>
    </div>
  </div>
);

const DailyChart = ({ points }: { points: TeamUsageDailyPoint[] }) => {
  if (points.length === 0) {
    return null;
  }
  const max = points.reduce((m, p) => Math.max(m, p.totalUsd), 0) || 1;
  return (
    <div className="rounded-xl border border-stone-200 bg-white px-3 py-2">
      <div className="text-[10px] font-semibold uppercase tracking-wide text-stone-400 mb-2">
        Daily spend
      </div>
      <div className="flex items-end gap-1 h-16">
        {points.map(p => {
          const inferenceHeight = (p.inferenceUsd / max) * 100;
          const integrationsHeight = (p.integrationsUsd / max) * 100;
          return (
            <div
              key={p.date}
              className="flex-1 h-full flex flex-col-reverse"
              title={`${p.date}: ${fmtUsd(p.totalUsd)}`}>
              <div className="bg-primary-400" style={{ height: `${inferenceHeight}%` }} />
              <div className="bg-amber-400" style={{ height: `${integrationsHeight}%` }} />
            </div>
          );
        })}
      </div>
      <div className="flex items-center gap-3 mt-1.5 text-[10px] text-stone-500">
        <span className="flex items-center gap-1">
          <span className="w-2 h-2 bg-primary-400 inline-block" /> Inference
        </span>
        <span className="flex items-center gap-1">
          <span className="w-2 h-2 bg-amber-400 inline-block" /> Integrations
        </span>
      </div>
    </div>
  );
};

const TopModels = ({ rows }: { rows: TeamUsageModelRow[] }) => (
  <div className="rounded-xl border border-stone-200 bg-white px-3 py-2">
    <div className="text-[10px] font-semibold uppercase tracking-wide text-stone-400 mb-1.5">
      Top models
    </div>
    {rows.length === 0 ? (
      <p className="text-[11px] text-stone-500">No inference usage this cycle.</p>
    ) : (
      <ul className="space-y-0.5">
        {rows.map((r, i) => (
          <li
            key={`${r.provider}::${r.model}::${i}`}
            className="flex items-center justify-between text-[11px]">
            <span className="text-stone-700 truncate mr-2">{r.model || r.provider}</span>
            <span className="text-stone-500 flex-shrink-0">
              {fmtUsd(r.spentUsd)} · {r.calls.toLocaleString()}
            </span>
          </li>
        ))}
      </ul>
    )}
  </div>
);

const TopIntegrations = ({ rows }: { rows: TeamUsageIntegrationRow[] }) => (
  <div className="rounded-xl border border-stone-200 bg-white px-3 py-2">
    <div className="text-[10px] font-semibold uppercase tracking-wide text-stone-400 mb-1.5">
      Top integrations
    </div>
    {rows.length === 0 ? (
      <p className="text-[11px] text-stone-500">No integration usage this cycle.</p>
    ) : (
      <ul className="space-y-0.5">
        {rows.map((r, i) => (
          <li
            key={`${r.provider}::${r.action}::${i}`}
            className="flex items-center justify-between text-[11px]">
            <span className="text-stone-700 truncate mr-2">
              {r.provider}
              {r.action ? ` · ${r.action}` : ''}
            </span>
            <span className="text-stone-500 flex-shrink-0">
              {fmtUsd(r.spentUsd)} · {r.calls.toLocaleString()}
            </span>
          </li>
        ))}
      </ul>
    )}
  </div>
);

export default InferenceBudget;
