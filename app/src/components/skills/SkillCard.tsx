import { useEffect, useMemo, useRef, useState } from 'react';

import SkillDebugModal from './SkillDebugModal';
import { DefaultIcon, STATUS_DISPLAY, type SkillListEntry } from './shared';
import {
  useSkillConnectionStatus,
  useSkillDataDirectoryStats,
  useSkillState,
} from '../../lib/skills/hooks';
import { skillManager } from '../../lib/skills/manager';
import type { SkillConnectionStatus, SkillHostConnectionState } from '../../lib/skills/types';
import {
  deriveSkillSyncSummaryText,
  deriveSkillSyncUiState,
  type SkillSyncStatsLike,
} from '../../pages/skillsSyncUi';

export interface UnifiedSkillCardProps {
  icon: React.ReactNode;
  title: string;
  description: string;
  statusDot?: string;
  statusLabel?: string;
  statusColor?: string;
  ctaLabel: string;
  ctaVariant?: 'primary' | 'sage' | 'amber';
  onCtaClick: () => void;
  secondaryActions?: Array<{
    label: string;
    icon: React.ReactNode;
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
  /** Used to generate data-testid on the CTA button: `skill-cta-{skillId}` */
  skillId?: string;
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
  skillId,
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
    <div className="flex items-center gap-3 p-3 rounded-xl bg-white border border-stone-100 hover:bg-stone-50 transition-colors">
      <div className="w-8 h-8 flex items-center justify-center text-stone-600 flex-shrink-0">
        {icon}
      </div>

      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold text-stone-900 truncate">{title}</span>
          {statusDot && (
            <div className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${statusDot}`} />
          )}
          {statusLabel && (
            <span className={`text-xs flex-shrink-0 ${statusColor ?? 'text-stone-400'}`}>
              {statusLabel}
            </span>
          )}
        </div>
        {description && (
          <p className="mt-1 text-xs leading-relaxed text-stone-600 line-clamp-2">{description}</p>
        )}
        {syncSummaryText && !syncProgress?.active && (
          <p className="text-[11px] text-stone-500 truncate mt-1">{syncSummaryText}</p>
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
                <div className="h-full w-1/2 rounded-full bg-primary-400/80 animate-pulse" />
              )}
            </div>
            {syncProgress.message && (
              <p className="text-[11px] text-primary-600 truncate mt-1">{syncProgress.message}</p>
            )}
            {syncProgress.metricsText && (
              <p className="text-[11px] text-stone-500 truncate mt-0.5">{syncProgress.metricsText}</p>
            )}
          </div>
        )}
      </div>

      <div className="flex items-center gap-1 flex-shrink-0">
        {secondaryActions && secondaryActions.length > 0 && (
          <div className="relative" ref={menuRef}>
            <button
              type="button"
              onClick={e => {
                e.stopPropagation();
                setMenuOpen(prev => !prev);
              }}
              className="w-7 h-7 flex items-center justify-center rounded-lg text-stone-400 hover:text-stone-700 hover:bg-stone-100 transition-colors"
              title="More actions">
              <svg className="w-3.5 h-3.5" fill="currentColor" viewBox="0 0 24 24">
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
          data-testid={skillId ? `skill-cta-${skillId}` : undefined}
          aria-label={skillId ? `${ctaLabel} ${skillId}` : undefined}
          onClick={e => {
            e.stopPropagation();
            onCtaClick();
          }}
          className={`flex-shrink-0 rounded-lg border px-3 py-1.5 text-[11px] font-medium transition-colors ${ctaStyle} ${ctaDisabled ? 'opacity-50 cursor-not-allowed' : ''}`}>
          {ctaLabel}
        </button>
      </div>
    </div>
  );
}

// Wrapper that handles all skill hooks and maps to UnifiedSkillCard
interface ThirdPartySkillCardProps {
  skill: SkillListEntry;
  onSetup: () => void;
  isInstalling?: boolean;
}

export function ThirdPartySkillCard({ skill, onSetup, isInstalling }: ThirdPartySkillCardProps) {
  const connectionStatus = useSkillConnectionStatus(skill.id);
  const statusDisplay = STATUS_DISPLAY[connectionStatus] ?? STATUS_DISPLAY.offline;
  const skillState = useSkillState<SkillHostConnectionState & Record<string, unknown>>(skill.id);
  const diskStats = useSkillDataDirectoryStats(skill.id, connectionStatus === 'connected');
  const syncStats = useMemo((): SkillSyncStatsLike | undefined => {
    const base: SkillSyncStatsLike = { ...diskStats };
    const sc = skillState?.syncCount;
    if (typeof sc === 'number' && Number.isFinite(sc)) base.syncCount = sc;
    const last =
      typeof skillState?.lastSyncAtMs === 'number'
        ? skillState.lastSyncAtMs
        : typeof skillState?.lastSyncTime === 'number'
          ? skillState.lastSyncTime
          : undefined;
    if (last != null && Number.isFinite(last)) base.lastSyncAtMs = last;
    if (
      base.syncCount == null &&
      base.lastSyncAtMs == null &&
      base.localDataBytes == null &&
      base.localFileCount == null
    ) {
      return undefined;
    }
    return base;
  }, [diskStats, skillState]);

  const [manualSyncing, setManualSyncing] = useState(false);
  const [debugOpen, setDebugOpen] = useState(false);
  const syncUi = useMemo(() => deriveSkillSyncUiState(skill.id, skillState), [skill.id, skillState]);
  const syncSummaryText = useMemo(
    () => deriveSkillSyncSummaryText(skillState, syncStats),
    [skillState, syncStats]
  );
  const isSyncing = manualSyncing || syncUi.isSyncing;

  const handleSync = async () => {
    setManualSyncing(true);
    try {
      await skillManager.triggerSync(skill.id);
    } catch (err) {
      console.error(`Sync failed for ${skill.id}:`, err);
    } finally {
      setManualSyncing(false);
    }
  };

  function statusDotClass(status: SkillConnectionStatus): string {
    switch (status) {
      case 'connected': return 'bg-sage-500';
      case 'connecting': return 'bg-amber-500 animate-pulse';
      case 'error': return 'bg-coral-500';
      default: return 'bg-stone-400';
    }
  }

  function ctaLabel(): string {
    if (isInstalling) return 'Enabling...';
    switch (connectionStatus) {
      case 'offline': return 'Enable';
      case 'setup_required': return 'Setup';
      case 'error': return 'Retry';
      default: return 'Manage';
    }
  }

  function ctaVariant(): 'primary' | 'sage' | 'amber' {
    if (connectionStatus === 'offline') return 'sage';
    if (connectionStatus === 'error') return 'amber';
    return 'primary';
  }

  const secondaryActions =
    connectionStatus === 'connected'
      ? [
          {
            label: 'Sync',
            icon: (
              <svg
                className={`w-3.5 h-3.5 ${isSyncing ? 'animate-spin' : ''}`}
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
                />
              </svg>
            ),
            onClick: () => void handleSync(),
            disabled: isSyncing,
            testId: `skill-sync-button-${skill.id}`,
          },
          {
            label: 'Debug',
            icon: (
              <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                />
              </svg>
            ),
            onClick: () => setDebugOpen(true),
            testId: `skill-debug-button-${skill.id}`,
          },
        ]
      : undefined;

  return (
    <>
      <UnifiedSkillCard
        icon={skill.icon ?? <DefaultIcon />}
        title={skill.name}
        description={skill.description}
        statusDot={statusDotClass(connectionStatus)}
        statusLabel={statusDisplay.text}
        statusColor={statusDisplay.color}
        ctaLabel={ctaLabel()}
        ctaVariant={ctaVariant()}
        ctaDisabled={isInstalling}
        onCtaClick={() => onSetup()}
        secondaryActions={secondaryActions}
        syncProgress={
          isSyncing
            ? {
                active: true,
                percent: syncUi.progressPercent ?? undefined,
                message: syncUi.progressMessage ?? undefined,
                metricsText: syncUi.metricsText ?? undefined,
              }
            : { active: false }
        }
        syncSummaryText={syncSummaryText ?? undefined}
      />
      {debugOpen && (
        <SkillDebugModal
          skillId={skill.id}
          skillName={skill.name}
          onClose={() => setDebugOpen(false)}
        />
      )}
    </>
  );
}

export default UnifiedSkillCard;
