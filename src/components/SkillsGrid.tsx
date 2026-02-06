import { invoke } from '@tauri-apps/api/core';
import { platform } from '@tauri-apps/plugin-os';
import { useEffect, useMemo, useState } from 'react';

import GoogleIcon from '../assets/icons/GoogleIcon';
import NotionIcon from '../assets/icons/notion.svg';
import TelegramIcon from '../assets/icons/telegram.svg';
import { useSkillConnectionStatus } from '../lib/skills/hooks';
import { skillManager } from '../lib/skills/manager';
import type { SkillConnectionStatus, SkillHostConnectionState } from '../lib/skills/types';
import { useAppSelector } from '../store/hooks';
import SkillSetupModal from './skills/SkillSetupModal';

// Map skill IDs to icons
const SKILL_ICONS: Record<string, React.ReactElement> = {
  telegram: <img src={TelegramIcon} alt="Telegram" className="w-5 h-5" />,
  email: <GoogleIcon className="w-5 h-5" />,
  notion: <img src={NotionIcon} alt="Notion" className="w-5 h-5" />,
  github: (
    <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
      <path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z" />
    </svg>
  ),
  otter: (
    <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
      <path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-2 15l-5-5 1.41-1.41L10 14.17l7.59-7.59L19 8l-9 9z" />
    </svg>
  ),
};

// Default icon for unknown skills
const DefaultIcon = () => (
  <div className="w-5 h-5 rounded-full bg-primary-500/20 flex items-center justify-center">
    <svg className="w-3 h-3 text-primary-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M12 6v6m0 0v6m0-6h6m-6 0H6"
      />
    </svg>
  </div>
);

// Status display text and colors
const STATUS_DISPLAY: Record<SkillConnectionStatus, { text: string; color: string }> = {
  connected: { text: 'Connected', color: 'text-sage-400' },
  connecting: { text: 'Connecting', color: 'text-amber-400' },
  not_authenticated: { text: 'Not Auth', color: 'text-amber-400' },
  disconnected: { text: 'Disconnected', color: 'text-stone-400' },
  error: { text: 'Error', color: 'text-coral-400' },
  offline: { text: 'Offline', color: 'text-stone-500' },
  setup_required: { text: 'Setup', color: 'text-primary-400' },
};

interface SkillRowProps {
  skillId: string;
  name: string;
  icon?: React.ReactElement;
  onConnect: (e: React.MouseEvent) => void;
}

function SkillRow({ skillId, name, icon, onConnect }: SkillRowProps) {
  const connectionStatus = useSkillConnectionStatus(skillId);
  const statusDisplay = STATUS_DISPLAY[connectionStatus] || STATUS_DISPLAY.offline;

  return (
    <tr
      onClick={onConnect}
      className="skill-row group hover:bg-stone-800/20 transition-all duration-300 cursor-pointer border-b border-stone-800/30 last:border-0">
      <td className="py-2.5 px-3">
        <div className="flex items-center gap-3">
          <div className="w-5 h-5 flex items-center justify-center text-white opacity-70 group-hover:opacity-100 transition-opacity flex-shrink-0">
            {icon || <DefaultIcon />}
          </div>
          <span className="text-sm text-white font-medium">{name}</span>
        </div>
      </td>
      <td className="py-2.5 px-3 text-right">
        <div className="flex items-center justify-end gap-1.5">
          <div
            className={`w-1.5 h-1.5 rounded-full ${
              connectionStatus === 'connected'
                ? 'bg-sage-400'
                : connectionStatus === 'connecting'
                  ? 'bg-amber-400 animate-pulse'
                  : connectionStatus === 'error'
                    ? 'bg-coral-400'
                    : 'bg-stone-600'
            }`}
          />
          <span className={`text-xs ${statusDisplay.color}`}>{statusDisplay.text}</span>
        </div>
      </td>
      <td className="py-2.5 px-3 w-8">
        <svg
          className="w-4 h-4 text-stone-500 group-hover:text-stone-300 transition-colors"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
        </svg>
      </td>
    </tr>
  );
}

interface SkillListEntry {
  id: string;
  name: string;
  description: string;
  icon?: React.ReactElement;
  hasSetup: boolean;
}

// Helper function to derive connection status (same logic as in hooks.ts)
function deriveConnectionStatus(
  lifecycleStatus: string | undefined,
  setupComplete: boolean | undefined,
  skillState: Record<string, unknown> | undefined
): SkillConnectionStatus {
  if (!lifecycleStatus || lifecycleStatus === 'installed') {
    return 'offline';
  }
  if (lifecycleStatus === 'error') {
    return 'error';
  }
  if (lifecycleStatus === 'setup_required' || lifecycleStatus === 'setup_in_progress') {
    return 'setup_required';
  }
  if (lifecycleStatus === 'starting') {
    return 'connecting';
  }
  const hostState = skillState as SkillHostConnectionState | undefined;
  if (!hostState) {
    if (setupComplete && lifecycleStatus === 'ready') {
      return 'connected';
    }
    return 'connecting';
  }
  const connStatus = hostState.connection_status;
  const authStatus = hostState.auth_status;
  if (connStatus === 'error' || authStatus === 'error') {
    return 'error';
  }
  if (connStatus === 'connected' && authStatus === 'authenticated') {
    return 'connected';
  }
  if (connStatus === 'connecting' || authStatus === 'authenticating') {
    return 'connecting';
  }
  if (connStatus === 'connected' && authStatus === 'not_authenticated') {
    return 'not_authenticated';
  }
  if (connStatus === 'disconnected') {
    return setupComplete ? 'disconnected' : 'setup_required';
  }
  return 'connecting';
}

// Priority order for sorting (lower number = higher priority)
const STATUS_PRIORITY: Record<SkillConnectionStatus, number> = {
  connected: 1,
  connecting: 2,
  not_authenticated: 3,
  disconnected: 4,
  setup_required: 5,
  offline: 6,
  error: 7,
};

// Contextual action button for skills in the management modal
function SkillActionButton({
  skill,
  connectionStatus,
  onOpenModal,
}: {
  skill: SkillListEntry;
  connectionStatus: SkillConnectionStatus;
  onOpenModal: () => void;
}) {
  const [loading, setLoading] = useState(false);

  const handleEnable = async (e: React.MouseEvent) => {
    e.stopPropagation();
    setLoading(true);
    try {
      await skillManager.startSkill({
        id: skill.id,
        name: skill.name,
        version: '0.0.0',
        description: skill.description,
        runtime: 'quickjs',
      });
      // If skill has setup, the manager will set setup_required status
      // and the grid will re-render with the "Setup" button
      if (skill.hasSetup) {
        onOpenModal();
      }
    } catch (err) {
      console.error(`Failed to enable ${skill.id}:`, err);
    } finally {
      setLoading(false);
    }
  };

  if (loading) {
    return (
      <div className="px-4 py-1.5 text-xs font-medium text-stone-400 flex-shrink-0 ml-3">
        <svg className="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
          <circle
            className="opacity-25"
            cx="12"
            cy="12"
            r="10"
            stroke="currentColor"
            strokeWidth="4"
          />
          <path
            className="opacity-75"
            fill="currentColor"
            d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
          />
        </svg>
      </div>
    );
  }

  // Offline / not started → Enable
  if (connectionStatus === 'offline') {
    return (
      <button
        onClick={handleEnable}
        className="px-4 py-1.5 text-xs font-medium text-sage-300 bg-sage-500/10 border border-sage-500/30 rounded-lg hover:bg-sage-500/20 transition-colors flex-shrink-0 ml-3">
        Enable
      </button>
    );
  }

  // Setup required → Setup
  if (connectionStatus === 'setup_required') {
    return (
      <button
        onClick={e => {
          e.stopPropagation();
          onOpenModal();
        }}
        className="px-4 py-1.5 text-xs font-medium text-primary-300 bg-primary-500/10 border border-primary-500/30 rounded-lg hover:bg-primary-500/20 transition-colors flex-shrink-0 ml-3">
        Setup
      </button>
    );
  }

  // Error → Retry
  if (connectionStatus === 'error') {
    return (
      <button
        onClick={handleEnable}
        className="px-4 py-1.5 text-xs font-medium text-amber-300 bg-amber-500/10 border border-amber-500/30 rounded-lg hover:bg-amber-500/20 transition-colors flex-shrink-0 ml-3">
        Retry
      </button>
    );
  }

  // Running / ready / connected → Configure
  return (
    <button
      onClick={e => {
        e.stopPropagation();
        onOpenModal();
      }}
      className="px-4 py-1.5 text-xs font-medium text-primary-300 bg-primary-500/10 border border-primary-500/30 rounded-lg hover:bg-primary-500/20 transition-colors flex-shrink-0 ml-3">
      Configure
    </button>
  );
}

export default function SkillsGrid() {
  const [skillsList, setSkillsList] = useState<SkillListEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [isMobile, setIsMobile] = useState(false);
  const [setupModalOpen, setSetupModalOpen] = useState(false);
  const [managementModalOpen, setManagementModalOpen] = useState(false);
  const [activeSkillId, setActiveSkillId] = useState<string | null>(null);
  const [activeSkillName, setActiveSkillName] = useState<string>('');
  const [activeSkillDescription, setActiveSkillDescription] = useState<string>('');
  const [activeSkillHasSetup, setActiveSkillHasSetup] = useState(false);

  // Get Redux state for sorting
  const skillsState = useAppSelector(state => state.skills.skills);
  const skillStates = useAppSelector(state => state.skills.skillStates);

  useEffect(() => {
    // Detect mobile platform
    const detectMobile = async () => {
      try {
        const currentPlatform = await platform();
        setIsMobile(currentPlatform === 'android' || currentPlatform === 'ios');
      } catch {
        // If we can't detect platform, assume desktop
        setIsMobile(false);
      }
    };
    detectMobile();

    // Load skills from the V8 runtime engine.
    const loadSkills = async () => {
      try {
        const manifests = await invoke<Array<Record<string, unknown>>>('runtime_discover_skills');

        console.log('manifests', manifests);

        // Validate skill names (underscores are reserved for tool namespacing)
        const validManifests = manifests.filter(m => {
          const id = m.id as string;
          if (id.includes('_')) {
            console.warn(
              `Skill "${id}" contains underscore and will be skipped. Skill names cannot contain underscores.`
            );
            return false;
          }
          return true;
        });

        const processed: SkillListEntry[] = validManifests.map(m => {
          const setup = m.setup as Record<string, unknown> | undefined;
          return {
            id: m.id as string,
            name:
              (m.name as string) ||
              (m.id as string).charAt(0).toUpperCase() + (m.id as string).slice(1),
            description: (m.description as string) || '',
            icon: SKILL_ICONS[m.id as string],
            hasSetup: !!(setup && setup.required),
          };
        });

        setSkillsList(processed);
        setLoading(false);
      } catch (error) {
        console.warn('Could not load skills from runtime:', error);
        setLoading(false);
      }
    };

    loadSkills();
  }, []);

  // Sort skills by connection status (connected first)
  const sortedSkillsList = useMemo(() => {
    return [...skillsList].sort((a, b) => {
      const skillA = skillsState[a.id];
      const skillB = skillsState[b.id];
      const stateA = skillStates[a.id];
      const stateB = skillStates[b.id];

      const statusA = deriveConnectionStatus(skillA?.status, skillA?.setupComplete, stateA);
      const statusB = deriveConnectionStatus(skillB?.status, skillB?.setupComplete, stateB);

      const priorityA = STATUS_PRIORITY[statusA] ?? 999;
      const priorityB = STATUS_PRIORITY[statusB] ?? 999;

      // If same priority, sort alphabetically by name
      if (priorityA === priorityB) {
        return a.name.localeCompare(b.name);
      }

      return priorityA - priorityB;
    });
  }, [skillsList, skillsState, skillStates]);

  // Show mobile-only message on mobile platforms
  if (!loading && isMobile) {
    return (
      <div className="animate-fade-up mt-4 mb-8 relative">
        <h3 className="text-sm font-semibold text-white mb-3 px-1 opacity-80 text-center">
          Skills
        </h3>
        <div className="glass rounded-xl p-4 text-center">
          <div className="flex flex-col items-center gap-2">
            <svg
              className="w-8 h-8 text-stone-400"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={1.5}
                d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
              />
            </svg>
            <p className="text-sm text-stone-400">Skills are available on desktop only</p>
            <p className="text-xs text-stone-500">
              Use the desktop app to configure and run skills
            </p>
          </div>
        </div>
      </div>
    );
  }

  // If loading or no skills on desktop, don't render
  if (loading || skillsList.length === 0) {
    return null;
  }

  const handleConnect = (skill: SkillListEntry) => {
    setActiveSkillId(skill.id);
    setActiveSkillName(skill.name);
    setActiveSkillDescription(skill.description);
    setActiveSkillHasSetup(skill.hasSetup);
    setSetupModalOpen(true);
  };

  return (
    <>
      <div className="animate-fade-up mt-4 mb-8 relative">
        <h3 className="text-sm font-semibold text-white mb-3 px-1 opacity-80 text-center">
          Available Skills
        </h3>
        <div
          className="glass rounded-xl overflow-hidden skills-table-container relative cursor-pointer"
          onClick={() => setManagementModalOpen(true)}>
          <div className="skills-table-scroll">
            <table className="w-full">
              <thead className="skills-table-header">
                <tr className="border-b border-stone-800/30">
                  <th className="py-2 px-3 text-left">
                    <span className="text-xs font-medium text-stone-400 uppercase tracking-wider">
                      Skill
                    </span>
                  </th>
                  <th className="py-2 px-3 text-right">
                    <span className="text-xs font-medium text-stone-400 uppercase tracking-wider">
                      Status
                    </span>
                  </th>
                  <th className="py-2 px-3 w-8"></th>
                </tr>
              </thead>
              <tbody className="skills-table-body">
                {sortedSkillsList.map(skill => (
                  <SkillRow
                    key={skill.id}
                    skillId={skill.id}
                    name={skill.name}
                    icon={skill.icon}
                    onConnect={e => {
                      e.stopPropagation();
                      handleConnect(skill);
                    }}
                  />
                ))}
              </tbody>
            </table>
          </div>
          {/* Hover overlay */}
          <div className="skills-table-overlay absolute inset-0 bg-black/80 flex items-center justify-center rounded-xl opacity-0 transition-opacity duration-200 pointer-events-none">
            <span className="text-sm font-medium text-white">Click to manage skills</span>
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
        <div
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 animate-fade-in"
          onClick={() => setManagementModalOpen(false)}>
          <div
            className="bg-stone-900 rounded-2xl max-w-2xl w-full max-h-[80vh] shadow-large border border-stone-700/50 flex flex-col overflow-hidden animate-slide-up"
            onClick={e => e.stopPropagation()}>
            {/* Sticky Header */}
            <div className="flex items-center justify-between p-6 pb-4 border-b border-stone-700/50 flex-shrink-0 bg-stone-900">
              <h2 className="text-xl font-semibold text-white">Manage Skills</h2>
              <button
                onClick={() => setManagementModalOpen(false)}
                className="text-stone-400 hover:text-white transition-colors">
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
            {/* Scrollable Content */}
            <div className="overflow-y-auto flex-1 p-6 pt-4">
              <div className="space-y-2">
                {sortedSkillsList.map(skill => {
                  const skillState = skillsState[skill.id];
                  const stateData = skillStates[skill.id];
                  const connectionStatus = deriveConnectionStatus(
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
                        onOpenModal={() => {
                          setActiveSkillId(skill.id);
                          setActiveSkillName(skill.name);
                          setActiveSkillDescription(skill.description);
                          setActiveSkillHasSetup(skill.hasSetup);
                          setSetupModalOpen(true);
                        }}
                      />
                    </div>
                  );
                })}
              </div>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
