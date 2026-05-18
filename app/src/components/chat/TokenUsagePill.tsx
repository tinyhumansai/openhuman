import { useUsageState } from '../../hooks/useUsageState';
import { useT } from '../../lib/i18n/I18nContext';
import { useAppSelector } from '../../store/hooks';
import { BILLING_DASHBOARD_URL } from '../../utils/links';
import { openUrl } from '../../utils/openUrl';

function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}K`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

interface PillSeverity {
  bg: string;
  text: string;
  ring: string;
  label: string;
}

function severityFromPct(pct: number): PillSeverity {
  if (pct >= 0.9) {
    return {
      bg: 'bg-coral-50 dark:bg-coral-500/15',
      text: 'text-coral-700 dark:text-coral-300',
      ring: 'ring-coral-200 dark:ring-coral-500/30',
      label: `${Math.round(pct * 100)}%`,
    };
  }
  if (pct >= 0.7) {
    return {
      bg: 'bg-amber-50 dark:bg-amber-500/15',
      text: 'text-amber-700 dark:text-amber-300',
      ring: 'ring-amber-200 dark:ring-amber-500/30',
      label: `${Math.round(pct * 100)}%`,
    };
  }
  return {
    bg: 'bg-sage-50 dark:bg-sage-500/15',
    text: 'text-sage-700 dark:text-sage-300',
    ring: 'ring-sage-200 dark:ring-sage-500/30',
    label: `${Math.round(pct * 100)}%`,
  };
}

const TokenUsagePill = () => {
  const { t } = useT();
  const sessionTokens = useAppSelector(state => state.chatRuntime.sessionTokenUsage);
  const { usagePct10h, usagePct7d, isAtLimit, isNearLimit, currentTier, teamUsage } =
    useUsageState();

  const totalTokens = sessionTokens.inputTokens + sessionTokens.outputTokens;
  const showSessionCounter = totalTokens > 0;

  const planPct = Math.max(usagePct10h, usagePct7d);
  const planSeverity = severityFromPct(planPct);
  const showPlanPill = teamUsage !== null;

  const planTitle = (() => {
    if (isAtLimit) return t('token.usageLimitReached');
    if (isNearLimit) return t('token.approachingLimit');
    return `${currentTier.toLowerCase()} ${t('token.planClickForDetails')}`;
  })();

  if (!showSessionCounter && !showPlanPill) return null;

  return (
    <div className="flex items-center gap-1.5 text-[11px] leading-none">
      {showSessionCounter ? (
        <span
          className="inline-flex items-center gap-1 rounded-full bg-stone-100 dark:bg-neutral-800 px-2 py-1 font-mono text-stone-600 dark:text-neutral-300 ring-1 ring-stone-200/60 dark:ring-neutral-700"
          title={t('token.sessionTokens')
            .replace('{in}', sessionTokens.inputTokens.toLocaleString())
            .replace('{out}', sessionTokens.outputTokens.toLocaleString())
            .replace('{turns}', String(sessionTokens.turns))}>
          <svg className="h-3 w-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M13 10V3L4 14h7v7l9-11h-7z"
            />
          </svg>
          {formatTokens(totalTokens)}
        </span>
      ) : null}
      {showPlanPill ? (
        <button
          type="button"
          onClick={() => {
            void openUrl(BILLING_DASHBOARD_URL);
          }}
          title={planTitle}
          className={`inline-flex items-center gap-1 rounded-full px-2 py-1 font-medium ring-1 transition-colors ${planSeverity.bg} ${planSeverity.text} ${planSeverity.ring} hover:opacity-80`}>
          {isAtLimit ? t('token.limit') : planSeverity.label}
        </button>
      ) : null}
    </div>
  );
};

export default TokenUsagePill;
