import { useT } from '../../lib/i18n/I18nContext';
import type { PlanTier } from '../../types/api';
import { BILLING_DASHBOARD_URL } from '../../utils/links';
import { openUrl } from '../../utils/openUrl';
import { PLANS } from '../settings/panels/billingHelpers';

interface UsageLimitModalProps {
  open: boolean;
  onClose: () => void;
  isBudgetExhausted: boolean;
  resetTime?: string | null;
  currentTier: PlanTier;
}

function formatResetTime(isoStr: string): string {
  const ms = new Date(isoStr).getTime() - Date.now();
  if (ms <= 0) return 'now';
  const mins = Math.ceil(ms / 60_000);
  if (mins < 60) return `in ${mins}m`;
  const hours = Math.floor(mins / 60);
  const remMins = mins % 60;
  if (hours < 24) return remMins > 0 ? `in ${hours}h ${remMins}m` : `in ${hours}h`;
  const days = Math.floor(hours / 24);
  const remHours = hours % 24;
  return remHours > 0 ? `in ${days}d ${remHours}h` : `in ${days}d`;
}

function getNextPlan(currentTier: PlanTier) {
  const currentIndex = PLANS.findIndex(p => p.tier === currentTier);
  if (currentIndex === -1 || currentIndex >= PLANS.length - 1) return null;
  return PLANS[currentIndex + 1];
}

export default function UsageLimitModal({
  open,
  onClose,
  isBudgetExhausted,
  resetTime,
  currentTier,
}: UsageLimitModalProps) {
  const { t } = useT();
  const nextPlan = getNextPlan(currentTier);

  if (!open) return null;

  const resetSuffix = resetTime
    ? ` ${t('upsell.usageLimit.resetsIn').replace('{time}', formatResetTime(resetTime))}`
    : '';
  const bodyText = isBudgetExhausted
    ? t('upsell.usageLimit.bodyBudget').replace('{reset}', resetSuffix)
    : t('upsell.usageLimit.bodyRate').replace('{reset}', resetSuffix);

  return (
    <div className="fixed inset-0 z-[9999] bg-black/30 backdrop-blur-sm flex items-center justify-center">
      <div className="bg-white dark:bg-neutral-900 rounded-2xl shadow-xl max-w-sm w-full mx-4 p-6">
        <div className="flex flex-col items-center text-center mb-4">
          <div className="w-12 h-12 rounded-full bg-amber-100 flex items-center justify-center mb-3">
            <svg
              className="w-6 h-6 text-amber-500"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
              />
            </svg>
          </div>
          <h2 className="text-lg font-semibold text-stone-900 dark:text-neutral-100">
            {t('upsell.usageLimit.heading')}
          </h2>
        </div>

        <p className="text-sm text-stone-600 dark:text-neutral-300 text-center mb-4">{bodyText}</p>

        {nextPlan && (
          <div className="rounded-xl bg-stone-50 dark:bg-neutral-800/60 border border-stone-200 dark:border-neutral-800 p-3 mb-5">
            <p className="text-xs font-medium text-stone-700 dark:text-neutral-200 mb-1">
              {t('upsell.usageLimit.planIncludes').replace('{plan}', nextPlan.name)}
            </p>
            <ul className="space-y-0.5">
              <li className="text-xs text-stone-600 dark:text-neutral-300">
                {t('upsell.usageLimit.perWindow').replace(
                  '{amount}',
                  nextPlan.fiveHourCapUsd.toFixed(2)
                )}
              </li>
              {nextPlan.weeklyBudgetUsd > 0 && (
                <li className="text-xs text-stone-600 dark:text-neutral-300">
                  {t('upsell.usageLimit.weeklyInference').replace(
                    '{amount}',
                    String(nextPlan.weeklyBudgetUsd)
                  )}
                </li>
              )}
            </ul>
          </div>
        )}

        <div className="flex flex-col gap-2">
          <button
            onClick={() => {
              onClose();
              void openUrl(BILLING_DASHBOARD_URL);
            }}
            className="w-full py-2.5 rounded-xl bg-primary-600 hover:bg-primary-500 text-white text-sm font-medium transition-colors">
            {t('upsell.usageLimit.upgradePlan')}
          </button>
          <button
            onClick={onClose}
            className="w-full py-2 text-sm text-stone-500 dark:text-neutral-400 hover:text-stone-700 dark:hover:text-neutral-200 transition-colors">
            {t('upsell.usageLimit.notNow')}
          </button>
        </div>
      </div>
    </div>
  );
}
