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
import { deriveConnectionStatus, useSkillConnectionStatus } from '../lib/skills/hooks';
import { skillManager } from '../lib/skills/manager';
import type { SkillConnectionStatus, SkillHostConnectionState } from '../lib/skills/types';
import { useAppSelector } from '../store/hooks';
import { IS_DEV } from '../utils/config';
import { deriveSkillSyncUiState } from './skillsSyncUi';

interface RegistryCatalogEntry {
  id: string;
  name: string;
  description: string;
  version: string;
  core: boolean;
  installed: boolean;
  update_available: boolean;
  can_uninstall: boolean;
}

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
}

function SkillCard({ skill, onSetup }: SkillCardProps) {
  const connectionStatus = useSkillConnectionStatus(skill.id);
  const statusDisplay = STATUS_DISPLAY[connectionStatus] || STATUS_DISPLAY.offline;
  const skillState = useAppSelector(state => state.skills.skillStates[skill.id]) as
    | (SkillHostConnectionState & Record<string, unknown>)
    | undefined;
  const [manualSyncing, setManualSyncing] = useState(false);
  const syncUi = useMemo(
    () => deriveSkillSyncUiState(skill.id, skillState),
    [skill.id, skillState]
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

// ─── Main Skills Page ───────────────────────────────────────────────────────

export default function Skills() {
  // Skills state
  const [skillsList, setSkillsList] = useState<SkillListEntry[]>([]);
  const [catalog, setCatalog] = useState<RegistryCatalogEntry[]>([]);
  const [catalogBusy, setCatalogBusy] = useState<Record<string, boolean>>({});
  const [skillsLoading, setSkillsLoading] = useState(true);
  const skillsState = useAppSelector(state => state.skills.skills);
  const skillStates = useAppSelector(state => state.skills.skillStates);

  // Modal state
  const [setupModalOpen, setSetupModalOpen] = useState(false);
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

        let catalogEntries: RegistryCatalogEntry[] = [];
        try {
          catalogEntries = await invoke<RegistryCatalogEntry[]>('registry_list_catalog');
        } catch (err) {
          console.warn('[Skills] failed to load registry catalog', err);
        }
        setCatalog(catalogEntries);

        const manifests = await invoke<Array<Record<string, unknown>>>('runtime_discover_skills');
        const validManifests = manifests.filter(m => {
          const id = m.id as string;
          if (id.includes('_')) return false;
          if (catalogEntries.length === 0) return true;
          return catalogEntries.some(c => c.id === id && c.installed);
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
              hasSetup: !!(setup && (setup.required || setup.oauth)),
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

  const refreshSkills = async () => {
    setSkillsLoading(true);
    try {
      const catalogEntries = await invoke<RegistryCatalogEntry[]>('registry_list_catalog');
      setCatalog(catalogEntries);
      const manifests = await invoke<Array<Record<string, unknown>>>('runtime_discover_skills');
      const validManifests = manifests.filter(m => {
        const id = m.id as string;
        if (id.includes('_')) return false;
        if (catalogEntries.length === 0) return true;
        return catalogEntries.some(c => c.id === id && c.installed);
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
            hasSetup: !!(setup && (setup.required || setup.oauth)),
          };
        })
        .filter(s => IS_DEV || !s.ignoreInProduction);

      setSkillsList(processed);
    } catch (err) {
      console.warn('[Skills] refresh failed', err);
    } finally {
      setSkillsLoading(false);
    }
  };

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

  const handleRegistryAction = async (
    skillId: string,
    action: 'install' | 'update' | 'uninstall'
  ) => {
    setCatalogBusy(prev => ({ ...prev, [skillId]: true }));
    try {
      if (action === 'install') {
        await invoke('registry_install_skill', { skill_id: skillId });
      } else if (action === 'update') {
        await invoke('registry_update_skill', { skill_id: skillId });
      } else {
        await invoke('registry_uninstall_skill', { skill_id: skillId });
      }
      await refreshSkills();
    } catch (err) {
      console.warn(`[Skills] registry ${action} failed for ${skillId}:`, err);
    } finally {
      setCatalogBusy(prev => ({ ...prev, [skillId]: false }));
    }
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
                    <SkillCard key={skill.id} skill={skill} onSetup={() => openSkillSetup(skill)} />
                  ))}
                </div>
              )}
            </div>

            <div className="animate-fade-up mt-8" style={{ animationDelay: '150ms' }}>
              <div className="mb-3">
                <h2 className="text-sm font-semibold text-white opacity-80">Skills Registry</h2>
              </div>
              {catalog.length === 0 ? (
                <div className="glass rounded-2xl p-6 text-center">
                  <p className="text-sm text-stone-500">No registry entries found</p>
                </div>
              ) : (
                <div className="space-y-2">
                  {catalog.map(entry => {
                    const busy = !!catalogBusy[entry.id];
                    return (
                      <div
                        key={entry.id}
                        className="flex items-center justify-between gap-3 p-3 rounded-xl bg-white/[0.03] border border-white/[0.06]">
                        <div className="min-w-0">
                          <div className="flex items-center gap-2">
                            <span className="text-sm font-medium text-white">{entry.name}</span>
                            <span
                              className={`text-[10px] px-1.5 py-0.5 rounded-md ${
                                entry.core
                                  ? 'bg-primary-500/15 text-primary-300'
                                  : 'bg-stone-700/50 text-stone-300'
                              }`}>
                              {entry.core ? 'Core' : 'Contributor'}
                            </span>
                            {entry.update_available && (
                              <span className="text-[10px] px-1.5 py-0.5 rounded-md bg-amber-500/15 text-amber-300">
                                Update Available
                              </span>
                            )}
                          </div>
                          <p className="text-xs text-stone-400 truncate">
                            {entry.description || 'No description'}
                          </p>
                        </div>
                        <div className="flex items-center gap-2">
                          {!entry.installed ? (
                            <button
                              disabled={busy}
                              onClick={() => handleRegistryAction(entry.id, 'install')}
                              className="px-3 py-1.5 text-xs font-medium text-sage-300 bg-sage-500/10 border border-sage-500/30 rounded-lg hover:bg-sage-500/20 disabled:opacity-50">
                              {busy ? 'Installing…' : 'Install'}
                            </button>
                          ) : (
                            <>
                              {entry.update_available && (
                                <button
                                  disabled={busy}
                                  onClick={() => handleRegistryAction(entry.id, 'update')}
                                  className="px-3 py-1.5 text-xs font-medium text-amber-300 bg-amber-500/10 border border-amber-500/30 rounded-lg hover:bg-amber-500/20 disabled:opacity-50">
                                  {busy ? 'Updating…' : 'Update'}
                                </button>
                              )}
                              {entry.can_uninstall && (
                                <button
                                  disabled={busy}
                                  onClick={() => handleRegistryAction(entry.id, 'uninstall')}
                                  className="px-3 py-1.5 text-xs font-medium text-coral-300 bg-coral-500/10 border border-coral-500/30 rounded-lg hover:bg-coral-500/20 disabled:opacity-50">
                                  {busy ? 'Removing…' : 'Uninstall'}
                                </button>
                              )}
                            </>
                          )}
                        </div>
                      </div>
                    );
                  })}
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
