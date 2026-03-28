import { useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import GoogleIcon from '../../../assets/icons/GoogleIcon';
import MetamaskIcon from '../../../assets/icons/metamask.svg';
import NotionIcon from '../../../assets/icons/notion.svg';
import TelegramIcon from '../../../assets/icons/telegram.svg';
import SkillSetupModal from '../../../components/skills/SkillSetupModal';
import { deriveConnectionStatus } from '../../../lib/skills/hooks';
import type { SkillConnectionStatus } from '../../../lib/skills/types';
import { useAppSelector } from '../../../store/hooks';

interface GetStartedStepProps {
  onComplete: (connectedSources: string[]) => void | Promise<void>;
}

interface SourceOption {
  id: string;
  skillId: string;
  name: string;
  description: string;
  icon: React.ReactElement;
}

const STATUS_BADGE: Record<SkillConnectionStatus, { label: string; classes: string }> = {
  connected: { label: 'Connected', classes: 'bg-sage-500/20 text-sage-400 border-sage-500/30' },
  connecting: {
    label: 'Connecting',
    classes: 'bg-amber-500/20 text-amber-400 border-amber-500/30',
  },
  not_authenticated: {
    label: 'Not Authenticated',
    classes: 'bg-amber-500/20 text-amber-400 border-amber-500/30',
  },
  disconnected: {
    label: 'Disconnected',
    classes: 'bg-stone-500/20 text-stone-400 border-stone-500/30',
  },
  error: { label: 'Error', classes: 'bg-coral-500/20 text-coral-400 border-coral-500/30' },
  offline: { label: 'Unavailable', classes: 'bg-stone-500/20 text-stone-400 border-stone-500/30' },
  setup_required: {
    label: 'Setup Required',
    classes: 'bg-primary-500/20 text-primary-400 border-primary-500/30',
  },
};

const SOURCE_OPTIONS: SourceOption[] = [
  {
    id: 'telegram',
    skillId: 'telegram',
    name: 'Telegram',
    description: 'Sync chats and message context for faster assistant responses.',
    icon: <img src={TelegramIcon} alt="Telegram" className="w-5 h-5" />,
  },
  {
    id: 'email',
    skillId: 'email',
    name: 'Google Email',
    description: 'Connect inbox workflows, summaries, and follow-up reminders.',
    icon: <GoogleIcon className="w-5 h-5" />,
  },
  {
    id: 'notion',
    skillId: 'notion',
    name: 'Notion',
    description: 'Bring docs and tasks into assistant context.',
    icon: <img src={NotionIcon} alt="Notion" className="w-5 h-5" />,
  },
  {
    id: 'wallet',
    skillId: 'wallet',
    name: 'Wallet',
    description: 'Enable secure on-chain workflows and portfolio actions.',
    icon: <img src={MetamaskIcon} alt="Wallet" className="w-5 h-5" />,
  },
];

const GetStartedStep = ({ onComplete }: GetStartedStepProps) => {
  const navigate = useNavigate();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [setupModalOpen, setSetupModalOpen] = useState(false);
  const [activeSkillId, setActiveSkillId] = useState<string | null>(null);
  const [activeSkillName, setActiveSkillName] = useState('');
  const [activeSkillDescription, setActiveSkillDescription] = useState('');
  const skills = useAppSelector(state => state.skills.skills);
  const skillStates = useAppSelector(state => state.skills.skillStates);

  const sources = useMemo(() => {
    return SOURCE_OPTIONS.map(option => {
      const skill = skills[option.skillId];
      const connectionStatus: SkillConnectionStatus = skill
        ? deriveConnectionStatus(skill.status, skill.setupComplete, skillStates[option.skillId])
        : 'offline';
      return { ...option, skill, connectionStatus };
    });
  }, [skillStates, skills]);

  const connectedSources = sources
    .filter(source => source.connectionStatus === 'connected')
    .map(source => source.id);

  const handleConnect = (source: (typeof sources)[number]) => {
    if (!source.skill) return;
    setActiveSkillId(source.skillId);
    setActiveSkillName(source.skill.manifest.name || source.name);
    setActiveSkillDescription(source.description);
    setSetupModalOpen(true);
  };

  const handleFinish = async () => {
    setError(null);
    setLoading(true);
    try {
      await onComplete(connectedSources);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Something went wrong. Please try again.');
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="glass rounded-3xl p-8 shadow-large animate-fade-up">
      <div className="text-center mb-4">
        <h1 className="text-xl font-bold mb-2">Connect Your Sources</h1>
        <p className="opacity-70 text-sm">
          Connect at least one source for richer context. You can also skip this and configure
          later.
        </p>
      </div>

      <div className="space-y-2 mb-4">
        {sources.map(source => {
          const badge = STATUS_BADGE[source.connectionStatus];
          return (
            <button
              key={source.id}
              type="button"
              onClick={() => handleConnect(source)}
              disabled={!source.skill}
              className={`w-full flex items-start space-x-3 p-3 bg-black/50 border border-stone-700 rounded-xl text-left transition-all ${
                source.skill
                  ? 'hover:border-stone-600 hover:shadow-medium'
                  : 'opacity-60 cursor-not-allowed'
              }`}>
              <div className="flex-shrink-0 mt-0.5">{source.icon}</div>
              <div className="flex-1 min-w-0">
                <div className="flex items-center justify-between gap-2">
                  <span className="font-medium text-sm">{source.name}</span>
                  <span className={`text-xs px-2 py-0.5 rounded border ${badge.classes}`}>
                    {badge.label}
                  </span>
                </div>
                <p className="opacity-70 text-xs mt-1">{source.description}</p>
              </div>
            </button>
          );
        })}
      </div>

      {connectedSources.length === 0 && (
        <p className="text-xs text-amber-300 mb-3">
          No sources connected yet. We will mark onboarding as partially incomplete so you can
          finish later.
        </p>
      )}

      {error && <p className="text-coral-400 text-sm mb-3 text-center">{error}</p>}

      <div className="space-y-2">
        <button
          onClick={handleFinish}
          disabled={loading}
          className="btn-primary w-full py-2.5 text-sm font-medium rounded-xl disabled:opacity-60 disabled:cursor-not-allowed">
          {loading ? 'Finishing…' : 'Finish Onboarding'}
        </button>
        <button
          type="button"
          onClick={() => navigate('/settings/connections')}
          className="w-full py-2.5 text-sm font-medium rounded-xl border border-stone-600 hover:border-stone-500 transition-colors">
          Open Connections Settings
        </button>
      </div>

      {setupModalOpen && activeSkillId && (
        <SkillSetupModal
          skillId={activeSkillId}
          skillName={activeSkillName}
          skillDescription={activeSkillDescription}
          onClose={() => {
            setSetupModalOpen(false);
            setActiveSkillId(null);
          }}
        />
      )}
    </div>
  );
};

export default GetStartedStep;
