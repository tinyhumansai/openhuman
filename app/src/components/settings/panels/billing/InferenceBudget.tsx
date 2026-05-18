import { useT } from '../../../../lib/i18n/I18nContext';
import type { TeamUsage } from '../../../../services/api/creditsApi';

interface InferenceBudgetProps {
  teamUsage: TeamUsage | null;
  isLoadingCredits: boolean;
}

const InferenceBudget = ({ teamUsage, isLoadingCredits }: InferenceBudgetProps) => {
  const { t } = useT();
  return (
    <div className="rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-3">
      <div className="flex items-center justify-between mb-2">
        <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
          {t('settings.billing.inferenceBudget.title')}
        </h3>
        {isLoadingCredits && (
          <span className="text-[10px] text-stone-500 dark:text-neutral-400">
            {t('common.loading')}
          </span>
        )}
        {teamUsage && !isLoadingCredits && (
          <span className="text-xs text-stone-400 dark:text-neutral-500">
            {teamUsage.cycleBudgetUsd > 0
              ? t('settings.billing.inferenceBudget.remaining')
                  .replace('{remaining}', (teamUsage.remainingUsd ?? 0).toFixed(2))
                  .replace('{budget}', (teamUsage.cycleBudgetUsd ?? 0).toFixed(2))
              : t('settings.billing.inferenceBudget.noRecurringBudget')}
          </span>
        )}
      </div>
      {teamUsage ? (
        teamUsage.cycleBudgetUsd > 0 ? (
          <>
            <div className="h-1.5 bg-stone-200 dark:bg-neutral-800 rounded-full overflow-hidden mb-2">
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
                <span className="text-[11px] text-stone-500 dark:text-neutral-400">
                  {t('settings.billing.inferenceBudget.tenHourCap')
                    .replace('{used}', (teamUsage.cycleLimit5hr ?? 0).toFixed(2))
                    .replace('{cap}', (teamUsage.fiveHourCapUsd ?? 0).toFixed(2))}
                </span>
              )}
              <span className="text-[11px] text-stone-500 dark:text-neutral-400 ml-auto">
                {t('settings.billing.inferenceBudget.cycleEnds').replace(
                  '{date}',
                  new Date(teamUsage.cycleEndsAt).toLocaleDateString('en-US')
                )}
              </span>
            </div>
            {teamUsage.remainingUsd <= 0 && (
              <p className="text-[11px] text-coral-400 mt-1.5">
                {t('settings.billing.inferenceBudget.exhausted')}
              </p>
            )}
          </>
        ) : (
          <div className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2.5">
            <p className="text-[11px] text-stone-600 dark:text-neutral-300">
              {t('settings.billing.inferenceBudget.noBudgetDesc')}
            </p>
          </div>
        )
      ) : isLoadingCredits ? (
        <div className="h-1.5 w-full rounded-full bg-stone-200 dark:bg-neutral-800 animate-pulse" />
      ) : (
        <p className="text-xs text-stone-500 dark:text-neutral-400">
          {t('settings.billing.inferenceBudget.loadError')}
        </p>
      )}
    </div>
  );
};

export default InferenceBudget;
