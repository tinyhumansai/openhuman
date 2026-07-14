/**
 * JoinPolicyToggle — 3-segment radio control for per-meeting join policy.
 *
 * Values: "auto" | "ask" | "skip"
 *
 * Phase 2: local state only. Phase 3 will add persistence.
 */
import { useT } from '../../lib/i18n/I18nContext';

export type JoinPolicy = 'auto' | 'ask' | 'skip';

export interface JoinPolicyToggleProps {
  value: JoinPolicy;
  onChange: (v: JoinPolicy) => void;
  disabled?: boolean;
  /** Compact variant: smaller text, tighter padding (default false). */
  compact?: boolean;
}

const SEGMENTS: JoinPolicy[] = ['auto', 'ask', 'skip'];

const KEY_MAP: Record<JoinPolicy, string> = {
  auto: 'skills.meetingBots.upcoming.auto',
  ask: 'skills.meetingBots.upcoming.ask',
  skip: 'skills.meetingBots.upcoming.skip',
};

export function JoinPolicyToggle({
  value,
  onChange,
  disabled = false,
  compact = false,
}: JoinPolicyToggleProps) {
  const { t } = useT();

  return (
    <div
      role="radiogroup"
      aria-label={t('skills.meetingBots.upcoming.joinPolicy')}
      className={[
        'inline-flex rounded-md border border-white/10 overflow-hidden',
        disabled ? 'opacity-50 pointer-events-none' : '',
      ]
        .filter(Boolean)
        .join(' ')}>
      {SEGMENTS.map(seg => {
        const isActive = seg === value;
        return (
          <button
            key={seg}
            type="button"
            role="radio"
            aria-checked={isActive}
            disabled={disabled}
            onClick={() => onChange(seg)}
            className={[
              'transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-500/25',
              compact ? 'px-2 py-0.5 text-xs' : 'px-2.5 py-1 text-xs',
              isActive
                ? 'bg-primary-500 text-content-inverted font-medium'
                : 'bg-transparent text-content-secondary hover:text-content hover:bg-surface-hover',
            ]
              .filter(Boolean)
              .join(' ')}>
            {t(KEY_MAP[seg])}
          </button>
        );
      })}
    </div>
  );
}
