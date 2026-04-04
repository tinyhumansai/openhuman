import { useState } from 'react';

import BinanceIcon from '../../../assets/icons/binance.svg';
import GoogleIcon from '../../../assets/icons/GoogleIcon';
import MetamaskIcon from '../../../assets/icons/metamask.svg';
import NotionIcon from '../../../assets/icons/notion.svg';
import SkillSetupModal from '../../../components/skills/SkillSetupModal';
import type { SkillConnectionStatus } from '../../../lib/skills/types';

interface ConnectStepProps {
  onNext: () => void;
}

interface ConnectOption {
  id: string;
  name: string;
  description: string;
  icon: React.ReactElement;
  comingSoon?: boolean;
  skillId?: string;
}

const STATUS_BADGE: Record<SkillConnectionStatus, { label: string; classes: string }> = {
  connected: { label: 'Connected', classes: 'bg-sage-500/20 text-sage-400' },
  connecting: { label: 'Connecting...', classes: 'bg-amber-500/20 text-amber-400' },
  not_authenticated: { label: 'Not Authenticated', classes: 'bg-amber-500/20 text-amber-400' },
  disconnected: { label: 'Disconnected', classes: 'bg-stone-500/20 text-stone-400' },
  error: { label: 'Error', classes: 'bg-coral-500/20 text-coral-400' },
  offline: { label: 'Offline', classes: 'bg-stone-500/20 text-stone-400' },
  setup_required: { label: 'Connect', classes: 'bg-primary-500/20 text-primary-400' },
};

function ConnectOptionRow({
  option,
  onConnect,
}: {
  option: ConnectOption;
  onConnect: (option: ConnectOption) => void;
}) {
  const connectionStatus = 'setup_required' as SkillConnectionStatus;
  const disabled = option.comingSoon;

  let badge: React.ReactElement;
  if (option.comingSoon) {
    badge = (
      <span className="text-xs text-stone-500 bg-stone-100 px-2 py-0.5 rounded">Coming Soon</span>
    );
  } else if (option.skillId) {
    const cfg = STATUS_BADGE[connectionStatus];
    badge = <span className={`text-xs px-2 py-0.5 rounded ${cfg.classes}`}>{cfg.label}</span>;
  } else {
    badge = (
      <span className="text-xs bg-primary-500/20 text-primary-400 px-2 py-0.5 rounded">
        Connect
      </span>
    );
  }

  return (
    <button
      disabled={disabled}
      onClick={() => onConnect(option)}
      className={`w-full flex items-start space-x-3 p-3 bg-white border border-stone-200 rounded-xl transition-all duration-200 text-left ${
        disabled ? 'opacity-50 cursor-not-allowed' : 'hover:border-stone-300 hover:shadow-medium'
      }`}>
      <div className="flex-shrink-0 mt-0.5">{option.icon}</div>
      <div className="flex-1 min-w-0">
        <div className="flex items-center justify-between">
          <span className="font-medium text-sm text-stone-900">{option.name}</span>
          {badge}
        </div>
        <p className="text-stone-600 text-xs mt-1">{option.description}</p>
      </div>
    </button>
  );
}

const ConnectStep = ({ onNext }: ConnectStepProps) => {
  const [setupModalOpen, setSetupModalOpen] = useState(false);
  const [activeSkillId, setActiveSkillId] = useState<string | null>(null);
  const [activeSkillName, setActiveSkillName] = useState<string>('');
  const [activeSkillDescription, setActiveSkillDescription] = useState<string>('');

  const connectOptions: ConnectOption[] = [
    {
      id: 'google',
      name: 'Google',
      description: 'Manage emails, contacts and calendar events',
      icon: <GoogleIcon />,
      comingSoon: true,
    },
    {
      id: 'notion',
      name: 'Notion',
      description: 'Manage tasks, documents and everything else in your Notion',
      icon: <img src={NotionIcon} alt="Notion" className="w-5 h-5" />,
      comingSoon: true,
    },
    {
      id: 'wallet',
      name: 'Web3 Wallet',
      description: 'Trade the trenches in a safe and secure way.',
      icon: <img src={MetamaskIcon} alt="Metamask" className="w-5 h-5" />,
      comingSoon: true,
    },
    {
      id: 'exchange',
      name: 'Crypto Trading Exchanges',
      description: 'Connect and make trades with deep insights.',
      icon: <img src={BinanceIcon} alt="Binance" className="w-5 h-5" />,
      comingSoon: true,
    },
  ];

  const handleConnect = (option: ConnectOption) => {
    if (option.comingSoon) return;
    if (option.skillId) {
      setActiveSkillId(option.skillId);
      setActiveSkillName(option.name);
      setActiveSkillDescription(option.description);
      setSetupModalOpen(true);
    }
  };

  return (
    <div className="rounded-2xl border border-stone-200 bg-white p-8 shadow-soft animate-fade-up">
      <div className="text-center mb-4">
        <h1 className="text-xl font-bold mb-2 text-stone-900">Connect Accounts</h1>
        <p className="text-stone-600 text-sm">
          The more accounts you connect, the more powerful the intelligence will be.
        </p>
      </div>

      <div className="space-y-3 mb-4">
        {connectOptions.map(option => (
          <ConnectOptionRow key={option.id} option={option} onConnect={handleConnect} />
        ))}
      </div>

      <div className="mt-4 p-4 bg-stone-50 rounded-xl border border-stone-200">
        <div className="flex items-start space-x-2">
          <div>
            <p className="font-medium text-sm text-stone-900">
              Remember everything is private &amp; encrypted!
            </p>
            <p className="text-stone-600 text-xs mt-1">
              All data and credentials are stored locally and follows a strict zero-data retention
              policy so you won't have to worry about anything getting leaked.
            </p>
          </div>
        </div>
      </div>

      <button
        onClick={onNext}
        className="btn-primary w-full py-2.5 text-sm font-medium rounded-xl mt-4">
        Continue
      </button>

      {/* Setup modal */}
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

export default ConnectStep;
