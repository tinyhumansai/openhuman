import { useNavigate } from 'react-router-dom';

import { useUsageState } from '../../hooks/useUsageState';
import { useAppSelector } from '../../store/hooks';

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
      bg: 'bg-coral-50',
      text: 'text-coral-700',
      ring: 'ring-coral-200',
      label: `${Math.round(pct * 100)}%`,
    };
  }
  if (pct >= 0.7) {
    return {
      bg: 'bg-amber-50',
      text: 'text-amber-700',
      ring: 'ring-amber-200',
      label: `${Math.round(pct * 100)}%`,
    };
  }
  return {
    bg: 'bg-sage-50',
    text: 'text-sage-700',
    ring: 'ring-sage-200',
    label: `${Math.round(pct * 100)}%`,
  };
}

const TokenUsagePill = () => {
  const navigate = useNavigate();
  const sessionTokens = useAppSelector(state => state.chatRuntime.sessionTokenUsage);
  const { usagePct10h, usagePct7d, isAtLimit, isNearLimit, currentTier, teamUsage } =
    useUsageState();

  const totalTokens = sessionTokens.inputTokens + sessionTokens.outputTokens;
  const showSessionCounter = totalTokens > 0;

  const planPct = Math.max(usagePct10h, usagePct7d);
  const planSeverity = severityFromPct(planPct);
  const showPlanPill = teamUsage !== null;

  const planTitle = (() => {
    if (isAtLimit) return 'Usage limit reached — click to top up';
    if (isNearLimit) return 'Approaching usage limit';
    return `${currentTier.toLowerCase()} plan — click for details`;
  })();

  if (!showSessionCounter && !showPlanPill) return null;

  return (
    <div className="flex items-center gap-1.5 text-[11px] leading-none">
      {showSessionCounter ? (
        <span
          className="inline-flex items-center gap-1 rounded-full bg-stone-100 px-2 py-1 font-mono text-stone-600 ring-1 ring-stone-200/60"
          title={`Session tokens: ${sessionTokens.inputTokens.toLocaleString()} in / ${sessionTokens.outputTokens.toLocaleString()} out across ${sessionTokens.turns} turn(s)`}>
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
          onClick={() => navigate('/settings/billing')}
          title={planTitle}
          className={`inline-flex items-center gap-1 rounded-full px-2 py-1 font-medium ring-1 transition-colors ${planSeverity.bg} ${planSeverity.text} ${planSeverity.ring} hover:opacity-80`}>
          {isAtLimit ? 'Limit' : planSeverity.label}
        </button>
      ) : null}
    </div>
  );
};

export default TokenUsagePill;
