import { useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import {
  DefaultIcon,
  SKILL_ICONS,
  SkillActionButton,
  type SkillListEntry,
  STATUS_DISPLAY,
} from '../components/skills/shared';
import SkillDebugModal from '../components/skills/SkillDebugModal';
import SkillSetupModal from '../components/skills/SkillSetupModal';
import {
  useAvailableSkills,
  useSkillConnectionStatus,
  useSkillDataDirectoryStats,
  useSkillState,
} from '../lib/skills/hooks';
import { skillManager } from '../lib/skills/manager';
import { installSkill } from '../lib/skills/skillsApi';
import type { SkillConnectionStatus, SkillHostConnectionState } from '../lib/skills/types';
import { IS_DEV } from '../utils/config';
import {
  deriveSkillSyncSummaryText,
  deriveSkillSyncUiState,
  type SkillSyncStatsLike,
} from './skillsSyncUi';

/** Status dot color for skill connection status */
function statusDotClass(status: SkillConnectionStatus): string {
  switch (status) {
    case 'connected':
      return 'bg-sage-500';
    case 'connecting':
      return 'bg-amber-500 animate-pulse';
    case 'error':
      return 'bg-coral-500';
    default:
      return 'bg-stone-400';
  }
}

// ─── Skill Card (used in the skills list) ───────────────────────────────────

interface SkillCardProps {
  skill: SkillListEntry;
  onSetup: () => void;
}

interface BuiltInSkillCardProps {
  title: string;
  description: string;
  route: string;
  icon: React.ReactNode;
  ctaLabel?: string;
}

function BuiltInSkillCard({
  title,
  description,
  route,
  icon,
  ctaLabel = 'Open settings',
}: BuiltInSkillCardProps) {
  const navigate = useNavigate();

  return (
    <button
      type="button"
      onClick={() => navigate(route)}
      className="w-full rounded-xl border border-stone-200 bg-stone-50 p-4 text-left transition-colors hover:bg-white hover:border-stone-300">
      <div className="flex items-start gap-3">
        <div className="flex h-10 w-10 flex-shrink-0 items-center justify-center rounded-xl bg-white text-stone-700 shadow-sm border border-stone-200">
          {icon}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center justify-between gap-3">
            <h2 className="text-sm font-semibold text-stone-900">{title}</h2>
            <span className="rounded-lg border border-primary-200 bg-primary-50 px-2.5 py-1 text-[11px] font-medium text-primary-700">
              {ctaLabel}
            </span>
          </div>
          <p className="mt-1 text-xs leading-relaxed text-stone-600">{description}</p>
        </div>
      </div>
    </button>
  );
}

function SkillCard({ skill, onSetup }: SkillCardProps) {
  const connectionStatus = useSkillConnectionStatus(skill.id);
  const statusDisplay = STATUS_DISPLAY[connectionStatus] || STATUS_DISPLAY.offline;
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
  const syncUi = useMemo(
    () => deriveSkillSyncUiState(skill.id, skillState),
    [skill.id, skillState]
  );
  const syncSummaryText = useMemo(
    () => deriveSkillSyncSummaryText(skillState, syncStats),
    [skillState, syncStats]
  );
  const isSyncing = manualSyncing || syncUi.isSyncing;

  const handleSync = async (e: React.MouseEvent) => {
    e.stopPropagation();
    setManualSyncing(true);
    try {
      await skillManager.triggerSync(skill.id);
    } catch (err) {
      console.error(`Sync failed for ${skill.id}:`, err);
    } finally {
      setManualSyncing(false);
    }
  };

  return (
    <div className="flex items-center gap-3 p-3 rounded-xl bg-white border border-stone-100 hover:bg-stone-50 transition-colors">
      {/* Icon */}
      <div className="w-8 h-8 flex items-center justify-center text-stone-600 flex-shrink-0">
        {skill.icon || <DefaultIcon />}
      </div>

      {/* Info */}
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold text-stone-900 truncate">{skill.name}</span>
          <div
            className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${statusDotClass(connectionStatus)}`}
          />
          <span className={`text-xs flex-shrink-0 ${statusDisplay.color}`}>
            {statusDisplay.text}
          </span>
        </div>
        {skill.description && (
          <p className="mt-1 text-xs leading-relaxed text-stone-600 line-clamp-2">
            {skill.description}
          </p>
        )}
        {syncSummaryText && (
          <p className="text-[11px] text-stone-500 truncate mt-1">{syncSummaryText}</p>
        )}
        {isSyncing && (
          <div className="mt-1.5">
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-stone-100">
              {syncUi.progressPercent != null ? (
                <div
                  className="h-full rounded-full bg-primary-400 transition-all duration-300"
                  style={{ width: `${syncUi.progressPercent}%` }}
                />
              ) : (
                <div className="h-full w-1/2 rounded-full bg-primary-400/80 animate-pulse" />
              )}
            </div>
            {syncUi.progressMessage && (
              <p className="text-[11px] text-primary-600 truncate mt-1">{syncUi.progressMessage}</p>
            )}
            {syncUi.metricsText && (
              <p className="text-[11px] text-stone-500 truncate mt-0.5">{syncUi.metricsText}</p>
            )}
          </div>
        )}
      </div>

      {/* Actions */}
      <div className="flex items-center gap-1 flex-shrink-0">
        {connectionStatus === 'connected' && (
          <>
            {/* Sync */}
            <button
              onClick={isSyncing ? undefined : handleSync}
              disabled={isSyncing}
              className="w-7 h-7 flex items-center justify-center rounded-lg text-stone-400 hover:text-stone-700 hover:bg-stone-100 transition-colors disabled:opacity-40"
              title="Sync">
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
            </button>
            {/* Settings */}
            <button
              onClick={e => {
                e.stopPropagation();
                onSetup();
              }}
              className="w-7 h-7 flex items-center justify-center rounded-lg text-stone-400 hover:text-stone-700 hover:bg-stone-100 transition-colors"
              title="Settings">
              <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
                />
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
                />
              </svg>
            </button>
            {/* Debug */}
            <button
              onClick={e => {
                e.stopPropagation();
                setDebugOpen(true);
              }}
              className="w-7 h-7 flex items-center justify-center rounded-lg text-stone-400 hover:text-amber-600 hover:bg-amber-50 transition-colors"
              title="Debug">
              <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                />
              </svg>
            </button>
          </>
        )}
        <SkillActionButton
          skill={skill}
          connectionStatus={connectionStatus}
          onOpenModal={onSetup}
        />
      </div>

      {debugOpen && (
        <SkillDebugModal
          skillId={skill.id}
          skillName={skill.name}
          onClose={() => setDebugOpen(false)}
        />
      )}
    </div>
  );
}

// ─── Main Skills Page ───────────────────────────────────────────────────────

export default function Skills() {
  const navigate = useNavigate();
  // Skills from registry via RPC
  const { skills: availableSkills, loading: skillsLoading } = useAvailableSkills();

  // Modal state
  const [setupModalOpen, setSetupModalOpen] = useState(false);
  const [activeSkillId, setActiveSkillId] = useState<string | null>(null);
  const [activeSkillName, setActiveSkillName] = useState('');
  const [activeSkillDescription, setActiveSkillDescription] = useState('');
  const [activeSkillHasSetup, setActiveSkillHasSetup] = useState(false);

  // Transform registry entries to SkillListEntry
  const skillsList: SkillListEntry[] = useMemo(() => {
    return availableSkills
      .filter(e => {
        if (e.id.includes('_')) return false;
        if (!IS_DEV && e.ignore_in_production) return false;
        return true;
      })
      .map(e => ({
        id: e.id,
        name: e.name || e.id.charAt(0).toUpperCase() + e.id.slice(1),
        description: e.description || '',
        icon: SKILL_ICONS[e.id],
        ignoreInProduction: e.ignore_in_production,
        hasSetup: !!(e.setup && e.setup.required),
      }));
  }, [availableSkills]);

  // Sort by name (connection status sorting will use the hook per-card)
  const sortedSkillsList = useMemo(() => {
    return [...skillsList].sort((a, b) => a.name.localeCompare(b.name));
  }, [skillsList]);

  const [installing, setInstalling] = useState<string | null>(null);

  const openSkillSetup = async (skill: SkillListEntry) => {
    try {
      setInstalling(skill.id);
      await installSkill(skill.id);
    } catch (err) {
      console.warn(`[Skills] install failed for ${skill.id}, continuing anyway:`, err);
    } finally {
      setInstalling(null);
    }

    setActiveSkillId(skill.id);
    setActiveSkillName(skill.name);
    setActiveSkillDescription(skill.description);
    setActiveSkillHasSetup(skill.hasSetup);
    setSetupModalOpen(true);
  };

  const builtInSkills = [
    {
      id: 'screen-intelligence',
      title: 'Screen Intelligence',
      description: 'Capture windows, summarize what is on screen, and feed useful context into memory.',
      route: '/settings/screen-intelligence',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.8}
            d="M3 5h18v12H3zM8 21h8m-4-4v4"
          />
        </svg>
      ),
    },
    {
      id: 'text-autocomplete',
      title: 'Text Auto-Complete',
      description: 'Suggest inline completions while you type and control where autocomplete is active.',
      route: '/settings/autocomplete',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.8}
            d="M4 7h16M4 12h10m-10 5h7m10 0l3 3m0 0l3-3m-3 3v-8"
          />
        </svg>
      ),
    },
    {
      id: 'voice-stt',
      title: 'Voice Speech To Text',
      description: 'Use the microphone for dictation and voice-driven chat with local speech recognition.',
      route: '/settings/local-model',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.8}
            d="M19 11a7 7 0 01-7 7m0 0a7 7 0 01-7-7m7 7v4m0 0H8m4 0h4m-4-8a3 3 0 01-3-3V5a3 3 0 116 0v6a3 3 0 01-3 3z"
          />
        </svg>
      ),
    },
  ];

  return (
    <div className="min-h-full">
      <div className="min-h-full flex flex-col">
        <div className="flex-1 flex items-start justify-center p-4 pt-6">
          <div className="max-w-lg w-full">
            <div className="mb-4 rounded-2xl border border-stone-200 bg-white p-3 shadow-soft animate-fade-up">
              <div className="px-1 pb-3 pt-1">
                <h2 className="text-sm font-semibold text-stone-900">Built-in Skills</h2>
                <p className="mt-1 text-xs text-stone-500">
                  Core desktop capabilities configured from settings.
                </p>
              </div>
              <div className="space-y-2">
                {builtInSkills.map(skill => (
                  <BuiltInSkillCard
                    key={skill.id}
                    title={skill.title}
                    description={skill.description}
                    route={skill.route}
                    icon={skill.icon}
                  />
                ))}
              </div>
            </div>

            {/* Main card */}
            <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-6 animate-fade-up">
              {/* Header */}
              <div className="mb-3 flex items-start justify-between gap-3">
                <div>
                  <h2 className="text-sm font-semibold text-stone-900">Connected Skills</h2>
                  <p className="mt-1 text-xs text-stone-500">
                    Third-party integrations and external data sources.
                  </p>
                </div>
                <button
                  type="button"
                  onClick={() => navigate('/settings/skills')}
                  className="text-xs font-medium text-stone-500 transition-colors hover:text-stone-800">
                  Skill settings
                </button>
              </div>

              {/* Skills list */}
              {skillsLoading || installing ? (
                <div className="py-8 text-center">
                  <p className="text-sm text-stone-400">
                    {installing ? `Installing ${installing}...` : 'Loading skills...'}
                  </p>
                </div>
              ) : sortedSkillsList.length === 0 ? (
                <div className="py-8 text-center">
                  <p className="text-sm text-stone-400">No skills discovered</p>
                </div>
              ) : (
                <div className="space-y-1">
                  {sortedSkillsList.map(skill => (
                    <SkillCard key={skill.id} skill={skill} onSetup={() => openSkillSetup(skill)} />
                  ))}
                </div>
              )}
            </div>
          </div>
        </div>
      </div>

      {/* Setup modal */}
      {setupModalOpen && activeSkillId && (
        <SkillSetupModal
          skillId={activeSkillId}
          skillName={activeSkillName}
          skillDescription={activeSkillDescription}
          hasSetup={activeSkillHasSetup}
          onClose={() => {
            setSetupModalOpen(false);
            setActiveSkillId(null);
          }}
        />
      )}
    </div>
  );
}
