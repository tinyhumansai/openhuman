import { useEffect, useRef, useState, type ReactNode } from 'react';

export interface UnifiedSkillCardProps {
  icon: ReactNode;
  title: string;
  description: string;
  statusDot?: string;
  statusLabel?: string;
  statusColor?: string;
  ctaLabel: string;
  ctaVariant?: 'primary' | 'sage' | 'amber';
  onCtaClick: () => void;
  badge?: ReactNode;
  secondaryActions?: Array<{
    label: string;
    icon: ReactNode;
    onClick: () => void;
    disabled?: boolean;
    testId?: string;
  }>;
  syncProgress?: {
    active: boolean;
    percent?: number;
    message?: string;
    metricsText?: string;
  };
  syncSummaryText?: string;
  ctaDisabled?: boolean;
}

const CTA_STYLES: Record<string, string> = {
  primary: 'border-primary-200 bg-primary-50 text-primary-700 hover:bg-primary-100',
  sage: 'border-sage-200 bg-sage-50 text-sage-700 hover:bg-sage-100',
  amber: 'border-amber-200 bg-amber-50 text-amber-700 hover:bg-amber-100',
};

export function UnifiedSkillCard({
  icon,
  title,
  description,
  statusDot,
  statusLabel,
  statusColor,
  ctaLabel,
  ctaVariant = 'primary',
  onCtaClick,
  secondaryActions,
  syncProgress,
  syncSummaryText,
  ctaDisabled,
}: UnifiedSkillCardProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menuOpen) return;
    const handleClick = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    };
    document.addEventListener('mousedown', handleClick);
    return () => document.removeEventListener('mousedown', handleClick);
  }, [menuOpen]);

  const ctaStyle = CTA_STYLES[ctaVariant] ?? CTA_STYLES.primary;

  return (
    <div className="flex items-center gap-3 rounded-xl border border-stone-100 bg-white p-3 transition-colors hover:bg-stone-50">
      <div className="flex h-8 w-8 flex-shrink-0 items-center justify-center text-stone-600">
        {icon}
      </div>

      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-semibold text-stone-900">{title}</span>
          {statusDot && <div className={`h-1.5 w-1.5 flex-shrink-0 rounded-full ${statusDot}`} />}
          {statusLabel && (
            <span className={`flex-shrink-0 text-xs ${statusColor ?? 'text-stone-400'}`}>
              {statusLabel}
            </span>
          )}
        </div>
        {description && (
          <p className="mt-1 line-clamp-2 text-xs leading-relaxed text-stone-600">{description}</p>
        )}
        {syncSummaryText && !syncProgress?.active && (
          <p className="mt-1 truncate text-[11px] text-stone-500">{syncSummaryText}</p>
        )}
        {syncProgress?.active && (
          <div className="mt-1.5">
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-stone-100">
              {syncProgress.percent != null ? (
                <div
                  className="h-full rounded-full bg-primary-400 transition-all duration-300"
                  style={{ width: `${syncProgress.percent}%` }}
                />
              ) : (
                <div className="h-full w-1/2 animate-pulse rounded-full bg-primary-400/80" />
              )}
            </div>
            {syncProgress.message && (
              <p className="mt-1 truncate text-[11px] text-primary-600">{syncProgress.message}</p>
            )}
            {syncProgress.metricsText && (
              <p className="mt-0.5 truncate text-[11px] text-stone-500">
                {syncProgress.metricsText}
              </p>
            )}
          </div>
        )}
      </div>

      <div className="flex flex-shrink-0 items-center gap-1">
        {secondaryActions && secondaryActions.length > 0 && (
          <div className="relative" ref={menuRef}>
            <button
              type="button"
              onClick={e => {
                e.stopPropagation();
                setMenuOpen(prev => !prev);
              }}
              className="flex h-7 w-7 items-center justify-center rounded-lg text-stone-400 transition-colors hover:bg-stone-100 hover:text-stone-700"
              title="More actions">
              <svg className="h-3.5 w-3.5" fill="currentColor" viewBox="0 0 24 24">
                <circle cx="5" cy="12" r="2" />
                <circle cx="12" cy="12" r="2" />
                <circle cx="19" cy="12" r="2" />
              </svg>
            </button>
            {menuOpen && (
              <div className="absolute right-0 top-8 z-10 w-36 rounded-xl border border-stone-200 bg-white py-1 shadow-md">
                {secondaryActions.map(action => (
                  <button
                    key={action.label}
                    type="button"
                    data-testid={action.testId}
                    disabled={action.disabled}
                    onClick={e => {
                      e.stopPropagation();
                      setMenuOpen(false);
                      action.onClick();
                    }}
                    className="flex w-full items-center gap-2 px-3 py-2 text-xs text-stone-700 hover:bg-stone-50 disabled:opacity-40">
                    {action.icon}
                    {action.label}
                  </button>
                ))}
              </div>
            )}
          </div>
        )}
        <button
          type="button"
          disabled={ctaDisabled}
          onClick={e => {
            e.stopPropagation();
            onCtaClick();
          }}
          className={`flex-shrink-0 rounded-lg border px-3 py-1.5 text-[11px] font-medium transition-colors ${ctaStyle} ${ctaDisabled ? 'cursor-not-allowed opacity-50' : ''}`}>
          {ctaLabel}
        </button>
      </div>
    </div>
  );
}

export default UnifiedSkillCard;
