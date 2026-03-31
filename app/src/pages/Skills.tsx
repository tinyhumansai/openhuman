import { useMemo, useState } from 'react';

import {
  DefaultIcon,
  SKILL_ICONS,
  SkillActionButton,
  type SkillListEntry,
  STATUS_DISPLAY,
} from '../components/skills/shared';
import SkillDebugModal from '../components/skills/SkillDebugModal';
import SkillSetupModal from '../components/skills/SkillSetupModal';
import { useAvailableSkills, useSkillConnectionStatus, useSkillState } from '../lib/skills/hooks';
import { skillManager } from '../lib/skills/manager';
import { installSkill } from '../lib/skills/skillsApi';
import type { SkillConnectionStatus, SkillHostConnectionState } from '../lib/skills/types';
import { IS_DEV } from '../utils/config';
import { deriveSkillSyncSummaryText, deriveSkillSyncUiState } from './skillsSyncUi';

/** Status dot color for skill connection status */
function statusDotClass(status: SkillConnectionStatus): string {
  switch (status) {
    case 'connected':
      return 'bg-sage-400';
    case 'connecting':
      return 'bg-amber-400 animate-pulse';
    case 'error':
      return 'bg-coral-400';
    default:
      return 'bg-stone-600';
  }
}

// ─── Skill Card (used in the skills list) ───────────────────────────────────

interface SkillCardProps {
  skill: SkillListEntry;
  onSetup: () => void;
}

function SkillCard({ skill, onSetup }: SkillCardProps) {
  const connectionStatus = useSkillConnectionStatus(skill.id);
  const statusDisplay = STATUS_DISPLAY[connectionStatus] || STATUS_DISPLAY.offline;
  const skillState = useSkillState<SkillHostConnectionState & Record<string, unknown>>(skill.id);
  const syncStats = undefined; // TODO: sync stats will come from RPC in future
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
    <div className="flex items-center gap-3 p-3 rounded-xl bg-white/[0.03] border border-white/[0.06] hover:bg-white/[0.06] transition-colors">
      {/* Icon */}
      <div className="w-8 h-8 flex items-center justify-center text-white opacity-70 flex-shrink-0">
        {skill.icon || <DefaultIcon />}
      </div>

      {/* Info */}
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-white truncate">{skill.name}</span>
          <div
            className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${statusDotClass(connectionStatus)}`}
          />
          <span className={`text-xs flex-shrink-0 ${statusDisplay.color}`}>
            {statusDisplay.text}
          </span>
        </div>
        {syncSummaryText && (
          <p className="text-[11px] text-stone-500 truncate mt-0.5">{syncSummaryText}</p>
        )}
        {isSyncing && (
          <div className="mt-1.5">
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-stone-800">
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
              <p className="text-[11px] text-primary-300 truncate mt-1">{syncUi.progressMessage}</p>
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
              className="w-7 h-7 flex items-center justify-center rounded-lg text-stone-400 hover:text-white hover:bg-white/10 transition-colors disabled:opacity-40"
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
              className="w-7 h-7 flex items-center justify-center rounded-lg text-stone-400 hover:text-white hover:bg-white/10 transition-colors"
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
              className="w-7 h-7 flex items-center justify-center rounded-lg text-stone-400 hover:text-amber-400 hover:bg-amber-500/10 transition-colors"
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

  return (
    <div className="min-h-full relative">
      <div className="relative z-10 min-h-full flex flex-col">
        <div className="flex-1 p-6">
          <div className="max-w-2xl mx-auto">
            {/* Header */}
            <div className="flex items-center justify-between mb-6">
              <h1 className="text-xl font-bold text-white">Skills</h1>
            </div>

            {/* Active Skills */}
            <div className="animate-fade-up" style={{ animationDelay: '100ms' }}>
              <div className="mb-3">
                <h2 className="text-sm font-semibold text-white opacity-80">Active Skills</h2>
              </div>

              {skillsLoading || installing ? (
                <div className="glass rounded-2xl p-6 text-center">
                  <p className="text-sm text-stone-500">
                    {installing ? `Installing ${installing}...` : 'Loading skills...'}
                  </p>
                </div>
              ) : sortedSkillsList.length === 0 ? (
                <div className="glass rounded-2xl p-6 text-center">
                  <p className="text-sm text-stone-500">No skills discovered</p>
                </div>
              ) : (
                <div className="space-y-2">
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
