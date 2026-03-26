import { invoke } from '@tauri-apps/api/core';
import { platform } from '@tauri-apps/plugin-os';
import { useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { deriveConnectionStatus, useSkillConnectionStatus } from '../lib/skills/hooks';
import { useAppSelector } from '../store/hooks';
import { IS_DEV } from '../utils/config';
import SelfEvolveModal from './skills/SelfEvolveModal';
import {
  DefaultIcon,
  SKILL_ICONS,
  type SkillListEntry,
  STATUS_DISPLAY,
  STATUS_PRIORITY,
} from './skills/shared';
import SkillSetupModal from './skills/SkillSetupModal';

/** Normalize a raw unified registry entry into a SkillListEntry for display. */
function normalizeUnifiedEntry(e: Record<string, unknown>): SkillListEntry {
  const setup = e.setup as { required?: boolean; oauth?: unknown } | undefined;
  // Treat both interactive setup steps and OAuth-only flows as "has setup"
  // so that clicking a skill (e.g. Gmail) opens the connection/setup wizard
  // instead of jumping straight to the management panel.
  const hasSetup =
    !!setup &&
    (setup.required === true ||
      // OAuth config means we still need a connection step in the wizard
      !!setup.oauth);

  return {
    id: e.id as string,
    name:
      (e.name as string) || (e.id as string).charAt(0).toUpperCase() + (e.id as string).slice(1),
    description: (e.description as string) || '',
    icon: SKILL_ICONS[e.id as string],
    ignoreInProduction: (e.ignoreInProduction as boolean) ?? false,
    hasSetup,
    skill_type: (e.skill_type as 'openhuman' | 'openclaw') ?? 'openhuman',
  };
}

interface SkillRowProps {
  skillId: string;
  name: string;
  icon?: React.ReactElement;
  skillType?: 'openhuman' | 'openclaw';
  onConnect: (e: React.MouseEvent) => void;
}

function SkillTypeBadge({ type }: { type?: string }) {
  if (!type) return null;
  const isOpenclaw = type === 'openclaw';
  return (
    <span
      className={`text-[10px] font-medium px-1.5 py-0.5 rounded-md ${
        isOpenclaw ? 'bg-sage-500/15 text-sage-400' : 'bg-primary-500/15 text-primary-400'
      }`}>
      {type}
    </span>
  );
}

function SkillRow({ skillId, name, icon, skillType, onConnect }: SkillRowProps) {
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
          <SkillTypeBadge type={skillType} />
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

export default function SkillsGrid() {
  const navigate = useNavigate();
  const [skillsList, setSkillsList] = useState<SkillListEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [isMobile, setIsMobile] = useState(false);
  const [generating, setGenerating] = useState(false);
  const [selfEvolveOpen, setSelfEvolveOpen] = useState(false);
  const [setupModalOpen, setSetupModalOpen] = useState(false);
  const [activeSkillId, setActiveSkillId] = useState<string | null>(null);
  const [activeSkillName, setActiveSkillName] = useState<string>('');
  const [activeSkillDescription, setActiveSkillDescription] = useState<string>('');
  const [activeSkillHasSetup, setActiveSkillHasSetup] = useState(false);
  const [activeSkillType, setActiveSkillType] = useState<'openhuman' | 'openclaw'>('openhuman');

  // Get Redux state for sorting
  const skillsState = useAppSelector(state => state.skills.skills);
  const skillStates = useAppSelector(state => state.skills.skillStates);

  // Load skills from the unified registry (covers both openhuman and openclaw types).
  // Extracted so it can be called after skill creation (e.g. from SelfEvolveModal).
  const refreshSkills = async () => {
    try {
      // Try unified registry first — it merges both skill types.
      const entries = await invoke<Array<Record<string, unknown>>>('unified_list_skills');

      const processed: SkillListEntry[] = entries
        .filter(e => {
          const id = e.id as string;
          if (id.includes('_')) {
            console.warn(
              `Skill "${id}" contains underscore and will be skipped. Skill IDs cannot contain underscores.`
            );
            return false;
          }
          return true;
        })
        .map(normalizeUnifiedEntry)
        .filter(s => IS_DEV || !s.ignoreInProduction);

      setSkillsList(processed);
    } catch {
      // Fallback to legacy runtime_discover_skills if unified registry isn't available.
      try {
        const manifests = await invoke<Array<Record<string, unknown>>>('runtime_discover_skills');
        const processed: SkillListEntry[] = manifests
          .filter(m => !(m.id as string).includes('_'))
          .map(m => {
            const setup = m.setup as { required?: boolean; oauth?: unknown } | undefined;
            const hasSetup =
              !!setup &&
              (setup.required === true ||
                // OAuth-only skills still need a setup/connect flow
                !!setup.oauth);
            return {
              id: m.id as string,
              name: (m.name as string) || (m.id as string),
              description: (m.description as string) || '',
              icon: SKILL_ICONS[m.id as string],
              ignoreInProduction: (m.ignoreInProduction as boolean) ?? false,
              hasSetup,
              skill_type: 'openhuman' as const,
            };
          })
          .filter(s => IS_DEV || !s.ignoreInProduction);
        setSkillsList(processed);
      } catch (err) {
        console.warn('Could not load skills:', err);
      }
    } finally {
      setLoading(false);
    }
  };

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
    refreshSkills();
  }, []);

  // Sort skills by connection status (connected first)
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

        // If same priority, sort alphabetically by name
        if (priorityA === priorityB) {
          return a.name.localeCompare(b.name);
        }

        return priorityA - priorityB;
      })
      .filter(s => IS_DEV || !s.ignoreInProduction);
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
    setActiveSkillType(skill.skill_type ?? 'openhuman');
    setSetupModalOpen(true);
  };

  return (
    <>
      <div className="animate-fade-up mt-4 mb-8 relative">
        <div className="flex items-center justify-between mb-3 px-1">
          <h3 className="text-sm font-semibold text-white opacity-80">Available Skills</h3>
          <div className="flex items-center gap-3">
            {/* Auto-Generate button — opens the self-evolving skill modal */}
            <button
              onClick={e => {
                e.stopPropagation();
                setSelfEvolveOpen(true);
              }}
              className="text-xs text-primary-400 hover:text-primary-300 transition-colors flex items-center gap-1">
              {/* Sparkle / robot icon */}
              <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M5 3l1.5 4.5L11 9l-4.5 1.5L5 15l-1.5-4.5L-1 9l4.5-1.5L5 3zM19 11l1 3 3 1-3 1-1 3-1-3-3-1 3-1 1-3z"
                />
              </svg>
              Auto-Generate
            </button>
            {/* Generate button — quick scaffold */}
            <button
              onClick={async e => {
                e.stopPropagation();
                setGenerating(true);
                try {
                  await invoke('unified_generate_skill', {
                    spec: {
                      name: `generated-demo-${Date.now()}`,
                      description: 'Auto-generated skill demonstrating the unified registry',
                      skill_type: 'openhuman',
                      tool_code:
                        'return { message: `Hello from generated skill! args=${JSON.stringify(args)}` };',
                    },
                  });
                  await refreshSkills();
                } catch (err) {
                  console.warn('Failed to generate skill:', err);
                } finally {
                  setGenerating(false);
                }
              }}
              className="text-xs text-primary-400 hover:text-primary-300 transition-colors flex items-center gap-1 disabled:opacity-50"
              disabled={generating}>
              {generating ? (
                <span className="opacity-60">Generating…</span>
              ) : (
                <>
                  <svg
                    className="w-3.5 h-3.5"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M12 4v16m8-8H4"
                    />
                  </svg>
                  Generate
                </>
              )}
            </button>
          </div>
        </div>
        <div
          className="glass rounded-xl overflow-hidden skills-table-container relative cursor-pointer"
          onClick={() => navigate('/skills')}>
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
                    skillType={skill.skill_type}
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
            <span className="text-sm font-medium text-white">View all skills</span>
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
          skillType={activeSkillType}
          onClose={() => {
            setSetupModalOpen(false);
            setActiveSkillId(null);
          }}
        />
      )}

      {/* Self-Evolve modal */}
      {selfEvolveOpen && (
        <SelfEvolveModal onClose={() => setSelfEvolveOpen(false)} onSkillCreated={refreshSkills} />
      )}
    </>
  );
}
