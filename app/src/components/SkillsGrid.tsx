import { useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useAvailableSkills, useSkillConnectionStatus } from '../lib/skills/hooks';
import { deriveSkillSyncSummaryText } from '../pages/skillsSyncUi';
import { IS_DEV } from '../utils/config';
import { DefaultIcon, SKILL_ICONS, type SkillListEntry, STATUS_DISPLAY } from './skills/shared';
import SkillSetupModal from './skills/SkillSetupModal';

interface SkillRowProps {
  skillId: string;
  name: string;
  icon?: React.ReactElement;
  skillType?: 'openhuman' | 'openclaw';
  syncSummaryText: string | null;
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

function SkillRow({ skillId, name, icon, skillType, syncSummaryText, onConnect }: SkillRowProps) {
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
      <td className="py-2.5 px-3 text-right">
        <span className="text-[11px] text-stone-500">{syncSummaryText ?? 'No syncs yet'}</span>
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
  const [setupModalOpen, setSetupModalOpen] = useState(false);
  const [activeSkillId, setActiveSkillId] = useState<string | null>(null);
  const [activeSkillName, setActiveSkillName] = useState<string>('');
  const [activeSkillDescription, setActiveSkillDescription] = useState<string>('');
  const [activeSkillHasSetup, setActiveSkillHasSetup] = useState(false);
  const [activeSkillType, setActiveSkillType] = useState<'openhuman' | 'openclaw'>('openhuman');

  const { skills: availableSkills, loading: registryLoading } = useAvailableSkills();

  // Derive SkillListEntry from registry
  const derivedSkillsList: SkillListEntry[] = useMemo(() => {
    return availableSkills
      .filter(e => {
        if (e.id.includes('_')) return false;
        if (!IS_DEV && e.ignore_in_production) return false;
        return true;
      })
      .map(e => {
        const setup = e.setup as { required?: boolean; oauth?: unknown } | undefined;
        const hasSetup = !!setup && (setup.required === true || !!setup.oauth);
        return {
          id: e.id,
          name: e.name || e.id,
          description: e.description || '',
          icon: SKILL_ICONS[e.id],
          ignoreInProduction: e.ignore_in_production,
          hasSetup,
          skill_type: 'openhuman' as const,
        };
      });
  }, [availableSkills]);

  // Update local state when registry loads (for compatibility with existing code below)
  if (!registryLoading && skillsList.length === 0 && derivedSkillsList.length > 0) {
    setSkillsList(derivedSkillsList);
    setLoading(false);
  }

  const sortedSkillsList = useMemo(() => {
    const list = skillsList.length > 0 ? skillsList : derivedSkillsList;
    return [...list]
      .sort((a, b) => a.name.localeCompare(b.name))
      .filter(s => IS_DEV || !s.ignoreInProduction);
  }, [skillsList, derivedSkillsList]);

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
                  <th className="py-2 px-3 text-right">
                    <span className="text-xs font-medium text-stone-400 uppercase tracking-wider">
                      Sync
                    </span>
                  </th>
                  <th className="py-2 px-3 w-8"></th>
                </tr>
              </thead>
              <tbody className="skills-table-body">
                {sortedSkillsList.map(skill => {
                  const syncSummaryText = deriveSkillSyncSummaryText(undefined, undefined);

                  return (
                    <SkillRow
                      key={skill.id}
                      skillId={skill.id}
                      name={skill.name}
                      icon={skill.icon}
                      skillType={skill.skill_type}
                      syncSummaryText={syncSummaryText}
                      onConnect={e => {
                        e.stopPropagation();
                        handleConnect(skill);
                      }}
                    />
                  );
                })}
              </tbody>
            </table>
          </div>
          <div className="skills-table-overlay absolute inset-0 bg-black/80 flex items-center justify-center rounded-xl opacity-0 transition-opacity duration-200 pointer-events-none">
            <span className="text-sm font-medium text-white">View all skills</span>
          </div>
        </div>
      </div>

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
    </>
  );
}
