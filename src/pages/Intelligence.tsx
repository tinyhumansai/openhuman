import { invoke } from '@tauri-apps/api/core';
import { platform } from '@tauri-apps/plugin-os';
import { useEffect, useMemo, useState } from 'react';

import {
  DefaultIcon,
  SKILL_ICONS,
  SkillActionButton,
  type SkillListEntry,
  STATUS_DISPLAY,
  STATUS_PRIORITY,
} from '../components/skills/shared';
import SkillSetupModal from '../components/skills/SkillSetupModal';
import { useIntelligenceStats } from '../hooks/useIntelligenceStats';
import { deriveConnectionStatus, useSkillConnectionStatus } from '../lib/skills/hooks';
import { skillManager } from '../lib/skills/manager';
import type { SkillConnectionStatus, SkillHostConnectionState } from '../lib/skills/types';
import { useAppSelector } from '../store/hooks';
import { IS_DEV } from '../utils/config';

/** Format large numbers: 1200 → "1.2K", 1200000 → "1.2M" */
function formatNumber(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

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
  onSync: () => void;
}

function SkillCard({ skill, onSetup }: SkillCardProps) {
  const connectionStatus = useSkillConnectionStatus(skill.id);
  const statusDisplay = STATUS_DISPLAY[connectionStatus] || STATUS_DISPLAY.offline;
  const skillState = useAppSelector(state => state.skills.skillStates[skill.id]) as
    | (SkillHostConnectionState & Record<string, unknown>)
    | undefined;
  const [syncing, setSyncing] = useState(false);

  const handleSync = async (e: React.MouseEvent) => {
    e.stopPropagation();
    setSyncing(true);
    try {
      await skillManager.triggerSync(skill.id);
    } catch (err) {
      console.error(`Sync failed for ${skill.id}:`, err);
    } finally {
      setSyncing(false);
    }
  };

  // Build subtitle from skill state (sync time, chat/message counts)
  const subtitleParts: string[] = [];
  if (skillState) {
    const chatCount = skillState.chat_count as number | undefined;
    const msgCount = skillState.message_count as number | undefined;
    const lastSync = skillState.last_sync as string | undefined;
    if (chatCount != null) subtitleParts.push(`${formatNumber(chatCount)} chats`);
    if (msgCount != null) subtitleParts.push(`${formatNumber(msgCount)} msgs`);
    if (lastSync) subtitleParts.push(`Synced ${lastSync}`);
  }

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
        {subtitleParts.length > 0 && (
          <p className="text-[11px] text-stone-500 truncate mt-0.5">{subtitleParts.join(' · ')}</p>
        )}
      </div>

      {/* Actions */}
      <div className="flex items-center gap-1 flex-shrink-0">
        {connectionStatus === 'connected' && (
          <>
            {/* Sync */}
            <button
              onClick={syncing ? undefined : handleSync}
              disabled={syncing}
              className="w-7 h-7 flex items-center justify-center rounded-lg text-stone-400 hover:text-white hover:bg-white/10 transition-colors disabled:opacity-40"
              title="Sync">
              <svg
                className={`w-3.5 h-3.5 ${syncing ? 'animate-spin' : ''}`}
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
          </>
        )}
        <SkillActionButton
          skill={skill}
          connectionStatus={connectionStatus}
          onOpenModal={onSetup}
        />
      </div>
    </div>
  );
}

// ─── Stat Card ──────────────────────────────────────────────────────────────

// function StatCard({ title, value, subtitle }: { title: string; value: string; subtitle?: string }) {
//   return (
//     <div className="glass rounded-2xl p-4">
//       <div className="text-xs font-medium text-stone-400 uppercase tracking-wider">{title}</div>
//       <div className="text-2xl font-bold text-white font-mono mt-1">{value}</div>
//       {subtitle && <div className="text-[11px] text-stone-500 mt-0.5">{subtitle}</div>}
//     </div>
//   );
// }

// ─── Main Intelligence Page ─────────────────────────────────────────────────

export default function Intelligence() {
  // const { sessions, memoryFiles, entities, entityError, aiStatus, isLoading } =
  //   useIntelligenceStats();
  const { aiStatus } = useIntelligenceStats();

  // Skills state
  const [skillsList, setSkillsList] = useState<SkillListEntry[]>([]);
  const [skillsLoading, setSkillsLoading] = useState(true);
  const skillsState = useAppSelector(state => state.skills.skills);
  const skillStates = useAppSelector(state => state.skills.skillStates);

  // Modal state
  const [setupModalOpen, setSetupModalOpen] = useState(false);
  const [managementModalOpen, setManagementModalOpen] = useState(false);
  const [activeSkillId, setActiveSkillId] = useState<string | null>(null);
  const [activeSkillName, setActiveSkillName] = useState('');
  const [activeSkillDescription, setActiveSkillDescription] = useState('');
  const [activeSkillHasSetup, setActiveSkillHasSetup] = useState(false);

  // Load skills
  useEffect(() => {
    const loadSkills = async () => {
      try {
        // Check if mobile
        try {
          const p = await platform();
          if (p === 'android' || p === 'ios') {
            setSkillsLoading(false);
            return;
          }
        } catch {
          // not Tauri env
        }

        const manifests = await invoke<Array<Record<string, unknown>>>('runtime_discover_skills');
        const validManifests = manifests.filter(m => {
          const id = m.id as string;
          if (id.includes('_')) return false;
          return true;
        });

        const processed: SkillListEntry[] = validManifests
          .map(m => {
            const setup = m.setup as Record<string, unknown> | undefined;
            return {
              id: m.id as string,
              name:
                (m.name as string) ||
                (m.id as string).charAt(0).toUpperCase() + (m.id as string).slice(1),
              description: (m.description as string) || '',
              icon: SKILL_ICONS[m.id as string],
              ignoreInProduction: (m.ignoreInProduction as boolean) ?? false,
              hasSetup: !!(setup && setup.required),
            };
          })
          .filter(s => IS_DEV || !s.ignoreInProduction);

        setSkillsList(processed);
      } catch {
        // Skills unavailable
      } finally {
        setSkillsLoading(false);
      }
    };
    loadSkills();
  }, []);

  // Sort skills by connection status
  const sortedSkillsList = useMemo(() => {
    return [...skillsList]
      .sort((a, b) => {
        const skillA = skillsState[a.id];
        const skillB = skillsState[b.id];
        const stateA = skillStates[a.id];
        const stateB = skillStates[b.id];

        const statusA = deriveConnectionStatus(skillA?.status, skillA?.setupComplete, stateA);
        const statusB = deriveConnectionStatus(skillB?.status, skillB?.setupComplete, stateB);

        const priorityA = STATUS_PRIORITY[statusA] ?? 999;
        const priorityB = STATUS_PRIORITY[statusB] ?? 999;

        if (priorityA === priorityB) return a.name.localeCompare(b.name);
        return priorityA - priorityB;
      })
      .filter(s => IS_DEV || !s.ignoreInProduction);
  }, [skillsList, skillsState, skillStates]);

  const openSkillSetup = (skill: SkillListEntry) => {
    setActiveSkillId(skill.id);
    setActiveSkillName(skill.name);
    setActiveSkillDescription(skill.description);
    setActiveSkillHasSetup(skill.hasSetup);
    setSetupModalOpen(true);
  };

  // AI status indicator
  const aiStatusLabel =
    aiStatus === 'ready'
      ? 'AI Ready'
      : aiStatus === 'initializing'
        ? 'Initializing...'
        : aiStatus === 'error'
          ? 'AI Error'
          : 'AI Idle';
  const aiStatusDot =
    aiStatus === 'ready'
      ? 'bg-sage-400'
      : aiStatus === 'initializing'
        ? 'bg-amber-400 animate-pulse'
        : aiStatus === 'error'
          ? 'bg-coral-400'
          : 'bg-stone-600';

  return (
    <div className="min-h-full relative">
      <div className="relative z-10 min-h-full flex flex-col">
        <div className="flex-1 p-6">
          <div className="max-w-2xl mx-auto">
            {/* Header */}
            <div className="flex items-center justify-between mb-6">
              <h1 className="text-xl font-bold text-white">Intelligence</h1>
              <div className="flex items-center gap-2">
                <div className={`w-2 h-2 rounded-full ${aiStatusDot}`} />
                <span className="text-xs text-stone-400">{aiStatusLabel}</span>
              </div>
            </div>

            {/* Stat Cards */}
            {/* <div className="grid grid-cols-2 md:grid-cols-3 gap-3 mb-6 animate-fade-up">
              <StatCard
                title="Sessions"
                value={isLoading ? '...' : sessions ? String(sessions.total) : '0'}
                subtitle="sessions"
              />
              <StatCard
                title="Memory"
                value={isLoading ? '...' : memoryFiles != null ? String(memoryFiles) : '0'}
                subtitle="files"
              />
              <StatCard
                title="Tokens"
                value={
                  isLoading ? '...' : sessions ? formatNumber(sessions.totalTokens) : '0'
                }
                subtitle="consumed"
              />
            </div> */}

            {/* Active Skills */}
            <div className="animate-fade-up" style={{ animationDelay: '100ms' }}>
              <div className="flex items-center justify-between mb-3">
                <h2 className="text-sm font-semibold text-white opacity-80">Active Skills</h2>
                <button
                  onClick={() => setManagementModalOpen(true)}
                  className="text-xs text-primary-400 hover:text-primary-300 transition-colors">
                  Manage Skills
                </button>
              </div>

              {skillsLoading ? (
                <div className="glass rounded-2xl p-6 text-center">
                  <p className="text-sm text-stone-500">Loading skills...</p>
                </div>
              ) : sortedSkillsList.length === 0 ? (
                <div className="glass rounded-2xl p-6 text-center">
                  <p className="text-sm text-stone-500">No skills discovered</p>
                </div>
              ) : (
                <div className="space-y-2">
                  {sortedSkillsList.map(skill => (
                    <SkillCard
                      key={skill.id}
                      skill={skill}
                      onSetup={() => openSkillSetup(skill)}
                      onSync={() => {
                        skillManager.triggerSync(skill.id).catch(console.error);
                      }}
                    />
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

      {/* Skills Management Modal */}
      {managementModalOpen && (
        <ManagementModal
          skills={sortedSkillsList}
          onClose={() => setManagementModalOpen(false)}
          onOpenSetup={openSkillSetup}
        />
      )}
    </div>
  );
}

// ─── Management Modal (reused from SkillsGrid pattern) ─────────────────────

function ManagementModal({
  skills,
  onClose,
  onOpenSetup,
}: {
  skills: SkillListEntry[];
  onClose: () => void;
  onOpenSetup: (skill: SkillListEntry) => void;
}) {
  const skillsState = useAppSelector(state => state.skills.skills);
  const skillStates = useAppSelector(state => state.skills.skillStates);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 animate-fade-in"
      onClick={onClose}>
      <div
        className="bg-stone-900 rounded-2xl max-w-2xl w-full max-h-[80vh] shadow-large border border-stone-700/50 flex flex-col overflow-hidden animate-slide-up"
        onClick={e => e.stopPropagation()}>
        {/* Header */}
        <div className="flex items-center justify-between p-6 pb-4 border-b border-stone-700/50 flex-shrink-0 bg-stone-900">
          <h2 className="text-xl font-semibold text-white">Manage Skills</h2>
          <button onClick={onClose} className="text-stone-400 hover:text-white transition-colors">
            <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M6 18L18 6M6 6l12 12"
              />
            </svg>
          </button>
        </div>
        {/* Content */}
        <div className="overflow-y-auto flex-1 p-6 pt-4">
          <div className="space-y-2">
            {skills.map(skill => {
              const skillState = skillsState[skill.id];
              const stateData = skillStates[skill.id];
              const connectionStatus: SkillConnectionStatus = deriveConnectionStatus(
                skillState?.status,
                skillState?.setupComplete,
                stateData
              );
              const statusDisplay = STATUS_DISPLAY[connectionStatus] || STATUS_DISPLAY.offline;

              return (
                <div
                  key={skill.id}
                  className="flex items-center justify-between p-3 rounded-lg bg-stone-800/30 border border-stone-700/30 hover:bg-stone-800/50 transition-colors">
                  <div className="flex items-center gap-3 flex-1 min-w-0">
                    <div className="w-8 h-8 flex items-center justify-center text-white opacity-70 flex-shrink-0">
                      {skill.icon || <DefaultIcon />}
                    </div>
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <div className="text-sm font-medium text-white">{skill.name}</div>
                        <span className={`text-xs ${statusDisplay.color}`}>
                          {statusDisplay.text}
                        </span>
                      </div>
                      <div className="text-xs text-stone-400">{skill.description}</div>
                    </div>
                  </div>
                  <SkillActionButton
                    skill={skill}
                    connectionStatus={connectionStatus}
                    onOpenModal={() => onOpenSetup(skill)}
                  />
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}
