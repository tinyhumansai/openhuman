import { useUsageState } from '../../hooks/useUsageState';
import { useT } from '../../lib/i18n/I18nContext';
import { LimitPill } from '../../pages/conversations/components/LimitPill';
import { formatResetTime } from '../../pages/conversations/utils/format';

/**
 * Self-contained cycle usage indicator for the composer toolbar.
 * Shows "CYCLE ——— 0%" pill with a hover tooltip showing spend/remaining.
 * Uses useUsageState() internally — no props needed.
 */
export default function CycleUsagePill() {
  const { t } = useT();
  const { usagePct, teamUsage, isLoading } = useUsageState();

  if (!isLoading && !teamUsage) return null;

  return (
    <div className="relative group">
      {teamUsage ? (
        <LimitPill label={t('chat.cycle')} usedPct={usagePct} />
      ) : (
        <span className="text-[10px] text-content-faint animate-pulse">{t('common.loading')}</span>
      )}
      {teamUsage && (
        <div className="absolute bottom-full left-0 mb-2 hidden group-hover:block z-50">
          <div className="bg-stone-900 text-white text-[10px] rounded-lg px-3 py-2 shadow-lg whitespace-nowrap space-y-1.5">
            <div className="flex items-center justify-between gap-4">
              <span className="text-content-faint">{t('chat.cycleSpent')}</span>
              <span>
                ${(teamUsage.cycleSpentUsd ?? 0).toFixed(2)} / $
                {(teamUsage.cycleBudgetUsd ?? 0).toFixed(2)}
              </span>
            </div>
            <div className="flex items-center justify-between gap-4">
              <span className="text-content-faint">{t('chat.cycleRemaining')}</span>
              <span>
                ${(teamUsage.remainingUsd ?? 0).toFixed(2)} {t('chat.left')}
                {teamUsage.cycleEndsAt && (
                  <span className="text-content-faint ml-1">
                    — {t('chat.resets')} {formatResetTime(teamUsage.cycleEndsAt)}
                  </span>
                )}
              </span>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
