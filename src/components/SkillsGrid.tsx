import { invoke } from '@tauri-apps/api/core';
import { platform } from '@tauri-apps/plugin-os';
import { useEffect, useMemo, useState } from 'react';

import { deriveConnectionStatus, useSkillConnectionStatus } from '../lib/skills/hooks';
import type { SkillConnectionStatus } from '../lib/skills/types';
import { useAppSelector } from '../store/hooks';
import { IS_DEV } from '../utils/config';
import {
  DefaultIcon,
  SKILL_ICONS,
  SkillActionButton,
  type SkillListEntry,
  STATUS_DISPLAY,
  STATUS_PRIORITY,
} from './skills/shared';
import SkillSetupModal from './skills/SkillSetupModal';

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
