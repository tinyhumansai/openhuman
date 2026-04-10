import type { TeamUsage } from '../../../../services/api/creditsApi';

interface InferenceBudgetProps {
  teamUsage: TeamUsage | null;
  isLoadingCredits: boolean;
}

const InferenceBudget = ({ teamUsage, isLoadingCredits }: InferenceBudgetProps) => (
  <div className="rounded-2xl border border-stone-200 bg-white p-3">
    <div className="flex items-center justify-between mb-2">
      <h3 className="text-sm font-semibold text-stone-900">Inference Budget</h3>
      {isLoadingCredits && <span className="text-[10px] text-stone-500">Loading…</span>}
      {teamUsage && !isLoadingCredits && (
        <span className="text-xs text-stone-400">
          {teamUsage.cycleBudgetUsd > 0
            ? `$${(teamUsage.remainingUsd ?? 0).toFixed(2)} / $${(teamUsage.cycleBudgetUsd ?? 0).toFixed(2)} remaining`
            : 'No recurring plan budget'}
        </span>
      )}
    </div>
    {teamUsage ? (
      teamUsage.cycleBudgetUsd > 0 ? (
        <>
          <div className="h-1.5 bg-stone-200 rounded-full overflow-hidden mb-2">
            <div
              className={`h-full rounded-full transition-all duration-300 ${
                teamUsage.remainingUsd <= 0
                  ? 'bg-coral-500'
                  : teamUsage.remainingUsd / teamUsage.cycleBudgetUsd < 0.2
                    ? 'bg-amber-500'
                    : 'bg-primary-500'
              }`}
              style={{
                width: `${Math.min(
                  100,
                  (teamUsage.remainingUsd / teamUsage.cycleBudgetUsd) * 100
                )}%`,
              }}
            />
          </div>
          <div className="mt-1 flex items-center justify-between">
            {((teamUsage.cycleLimit5hr ?? 0) > 0 || (teamUsage.fiveHourCapUsd ?? 0) > 0) && (
              <span className="text-[11px] text-stone-500">
                10-hour cap: ${(teamUsage.cycleLimit5hr ?? 0).toFixed(2)} / $
                {(teamUsage.fiveHourCapUsd ?? 0).toFixed(2)}
              </span>
            )}
            <span className="text-[11px] text-stone-500 ml-auto">
              Cycle ends {new Date(teamUsage.cycleEndsAt).toLocaleDateString('en-US')}
            </span>
          </div>
          {teamUsage.remainingUsd <= 0 && (
            <p className="text-[11px] text-coral-400 mt-1.5">
              Included subscription usage is exhausted. Top up credits to continue using AI features
              without waiting for the next cycle.
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
      )
    ) : isLoadingCredits ? (
      <div className="h-1.5 w-full rounded-full bg-stone-200 animate-pulse" />
    ) : (
      <p className="text-xs text-stone-500">Unable to load usage data</p>
    )}
  </div>
);

export default InferenceBudget;
