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
          ${teamUsage.remainingUsd.toFixed(2)} / ${teamUsage.cycleBudgetUsd.toFixed(2)} remaining
        </span>
      )}
    </div>
    {teamUsage ? (
      <>
        <div className="h-1.5 bg-stone-700/60 rounded-full overflow-hidden mb-2">
          <div
            className={`h-full rounded-full transition-all duration-300 ${
              teamUsage.remainingUsd <= 0
                ? 'bg-coral-500'
                : teamUsage.remainingUsd / teamUsage.cycleBudgetUsd < 0.2
                  ? 'bg-amber-500'
                  : 'bg-primary-500'
            }`}
            style={{
              width: `${Math.min(100, (teamUsage.remainingUsd / teamUsage.cycleBudgetUsd) * 100)}%`,
            }}
          />
        </div>
        <div className="mt-1 flex items-center justify-between">
          <span className="text-[11px] text-stone-500">
            5-hour cap: ${teamUsage.cycleLimit5hr.toFixed(2)} / $
            {teamUsage.fiveHourCapUsd.toFixed(2)}
          </span>
          <span className="text-[11px] text-stone-500">
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
    ) : isLoadingCredits ? (
      <div className="h-1.5 w-full rounded-full bg-stone-700/60 animate-pulse" />
    ) : (
      <p className="text-xs text-stone-500">Unable to load usage data</p>
    )}
  </div>
);

export default InferenceBudget;
