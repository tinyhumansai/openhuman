import { useMemo, useState } from 'react';

import GoogleIcon from '../../../assets/icons/GoogleIcon';
import MetamaskIcon from '../../../assets/icons/metamask.svg';
import NotionIcon from '../../../assets/icons/notion.svg';
import TelegramIcon from '../../../assets/icons/telegram.svg';
import SkillSetupModal from '../../../components/skills/SkillSetupModal';
import { useAllSkillSnapshots } from '../../../lib/skills/hooks';
import type { SkillConnectionStatus } from '../../../lib/skills/types';

interface SkillsStepProps {
  onComplete: (connectedSources: string[]) => void | Promise<void>;
  onBack?: () => void;
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

const SkillsStep = ({ onComplete, onBack }: SkillsStepProps) => {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [setupModalOpen, setSetupModalOpen] = useState(false);
  const [activeSkillId, setActiveSkillId] = useState<string | null>(null);
  const [activeSkillName, setActiveSkillName] = useState('');
  const [activeSkillDescription, setActiveSkillDescription] = useState('');
  const snapshots = useAllSkillSnapshots();

  const sources = useMemo(() => {
    return SOURCE_OPTIONS.map(option => {
      const snap = snapshots.find(s => s.skill_id === option.skillId);
      const connectionStatus: SkillConnectionStatus = snap
        ? (snap.connection_status as SkillConnectionStatus) || 'offline'
        : 'offline';
      return { ...option, snap, connectionStatus };
    });
  }, [snapshots]);

  const connectedSources = sources
    .filter(source => source.connectionStatus === 'connected')
    .map(source => source.id);

  const handleConnect = (source: (typeof sources)[number]) => {
    setActiveSkillId(source.skillId);
    setActiveSkillName(source.snap?.name || source.name);
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
    <div className="rounded-3xl border border-stone-700 bg-stone-900 p-8 shadow-large animate-fade-up">
      <div className="text-center mb-4">
        <h1 className="text-xl font-bold mb-2">Install Skills</h1>
        <p className="opacity-70 text-sm">
          Connect integrations to give OpenHuman richer context. You can skip this and set them up
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
              disabled={!source.snap}
              className={`w-full flex items-start space-x-3 p-3 bg-black/50 border border-stone-700 rounded-xl text-left transition-all ${
                source.snap
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
          No skills connected yet. You can finish and configure later.
        </p>
      )}

      {error && <p className="text-coral-400 text-sm mb-3 text-center">{error}</p>}

      <div className="flex gap-2">
        {onBack && (
          <button
            onClick={onBack}
            disabled={loading}
            className="py-2.5 px-4 text-sm font-medium rounded-xl bg-stone-800 hover:bg-stone-700 transition-colors disabled:opacity-60">
            Back
          </button>
        )}
        <button
          onClick={handleFinish}
          disabled={loading}
          className="btn-primary flex-1 py-2.5 text-sm font-medium rounded-xl disabled:opacity-60 disabled:cursor-not-allowed">
          {loading ? 'Finishing...' : 'Finish Setup'}
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

export default SkillsStep;
