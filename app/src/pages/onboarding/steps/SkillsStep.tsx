import { useMemo, useState } from 'react';

import {
  DefaultIcon,
  SKILL_ICONS,
  SkillActionButton,
  type SkillListEntry,
  STATUS_DISPLAY,
} from '../../../components/skills/shared';
import SkillSetupModal from '../../../components/skills/SkillSetupModal';
import { useAvailableSkills, useSkillConnectionStatus } from '../../../lib/skills/hooks';
import { installSkill } from '../../../lib/skills/skillsApi';
import type { SkillConnectionStatus } from '../../../lib/skills/types';
import { IS_DEV } from '../../../utils/config';
import OnboardingNextButton from '../components/OnboardingNextButton';

interface SkillsStepProps {
  onNext: (connectedSources: string[]) => void | Promise<void>;
  onBack?: () => void;
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

function SkillRow({ skill, onSetup }: { skill: SkillListEntry; onSetup: () => void }) {
  const connectionStatus = useSkillConnectionStatus(skill.id);
  const statusDisplay = STATUS_DISPLAY[connectionStatus] || STATUS_DISPLAY.offline;

  return (
    <div className="flex items-center gap-3 p-3 rounded-xl border border-stone-700 bg-stone-900 hover:border-stone-600 transition-colors">
      <div className="w-6 h-6 flex items-center justify-center text-white opacity-70 flex-shrink-0">
        {skill.icon || <DefaultIcon />}
      </div>
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
        {skill.description && (
          <p className="text-xs opacity-50 mt-0.5 truncate">{skill.description}</p>
        )}
      </div>
      <SkillActionButton skill={skill} connectionStatus={connectionStatus} onOpenModal={onSetup} />
    </div>
  );
}

const SkillsStep = ({ onNext, onBack: _onBack }: SkillsStepProps) => {
  const { skills: availableSkills, loading: skillsLoading } = useAvailableSkills();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [installing, setInstalling] = useState<string | null>(null);
  const [setupModalOpen, setSetupModalOpen] = useState(false);
  const [activeSkillId, setActiveSkillId] = useState<string | null>(null);
  const [activeSkillName, setActiveSkillName] = useState('');
  const [activeSkillDescription, setActiveSkillDescription] = useState('');
  const [activeSkillHasSetup, setActiveSkillHasSetup] = useState(false);

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

  const sortedSkills = useMemo(() => {
    return [...skillsList].sort((a, b) => a.name.localeCompare(b.name));
  }, [skillsList]);

  const openSkillSetup = async (skill: SkillListEntry) => {
    try {
      setInstalling(skill.id);
      await installSkill(skill.id);
    } catch (err) {
      console.warn(`[SkillsStep] install failed for ${skill.id}, continuing:`, err);
    } finally {
      setInstalling(null);
    }

    setActiveSkillId(skill.id);
    setActiveSkillName(skill.name);
    setActiveSkillDescription(skill.description);
    setActiveSkillHasSetup(skill.hasSetup);
    setSetupModalOpen(true);
  };

  const handleFinish = async () => {
    setError(null);
    setLoading(true);
    try {
      const connectedIds = sortedSkills.map(s => s.id);
      await onNext(connectedIds);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Something went wrong. Please try again.');
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="rounded-3xl border border-stone-700 bg-stone-900 p-8 shadow-large animate-fade-up">
      <div className="text-center mb-4">
        <h1 className="text-xl font-bold mb-2">Install Skills</h1>
        <p className="opacity-70 text-sm">
          Skills give OpenHuman richer context & access to your workflow. All data consumed by
          skills is saved and processed locally. You can connect as many skills as you want.
        </p>
      </div>

      <div className="space-y-2 mb-4 max-h-[380px] overflow-y-auto pr-1">
        {skillsLoading || installing ? (
          <div className="rounded-2xl p-6 text-center">
            <p className="text-sm text-stone-500">
              {installing ? `Installing ${installing}...` : 'Loading skills...'}
            </p>
          </div>
        ) : sortedSkills.length === 0 ? (
          <div className="rounded-2xl p-6 text-center">
            <p className="text-sm text-stone-500">No skills discovered</p>
          </div>
        ) : (
          sortedSkills.map(skill => (
            <SkillRow key={skill.id} skill={skill} onSetup={() => openSkillSetup(skill)} />
          ))
        )}
      </div>

      {error && <p className="text-coral-400 text-sm mb-3 text-center">{error}</p>}

      <OnboardingNextButton onClick={handleFinish} loading={loading} loadingLabel="Loading..." />

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
};

export default SkillsStep;
