import { useState } from 'react';

import BinanceIcon from '../../../assets/icons/binance.svg';
import GoogleIcon from '../../../assets/icons/GoogleIcon';
import MetamaskIcon from '../../../assets/icons/metamask.svg';
import NotionIcon from '../../../assets/icons/notion.svg';
import TelegramIcon from '../../../assets/icons/telegram.svg';
import { useSkillConnectionStatus } from '../../../lib/skills/hooks';
import type { SkillConnectionStatus } from '../../../lib/skills/types';
import SkillSetupModal from '../../skills/SkillSetupModal';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

interface ConnectOption {
  id: string;
  name: string;
  description: string;
  icon: React.ReactElement;
  comingSoon?: boolean;
  skillId?: string;
}

// ---------------------------------------------------------------------------
// Status badge mapping
// ---------------------------------------------------------------------------

const STATUS_BADGE_CONFIG: Record<
  SkillConnectionStatus,
  { label: string; bg: string; text: string; border: string }
> = {
  connected: {
    label: 'Connected',
    bg: 'bg-sage-500/20',
    text: 'text-sage-400',
    border: 'border-sage-500/30',
  },
  connecting: {
    label: 'Connecting...',
    bg: 'bg-amber-500/20',
    text: 'text-amber-400',
    border: 'border-amber-500/30',
  },
  not_authenticated: {
    label: 'Not Authenticated',
    bg: 'bg-amber-500/20',
    text: 'text-amber-400',
    border: 'border-amber-500/30',
  },
  disconnected: {
    label: 'Disconnected',
    bg: 'bg-stone-500/20',
    text: 'text-stone-400',
    border: 'border-stone-500/30',
  },
  error: {
    label: 'Error',
    bg: 'bg-coral-500/20',
    text: 'text-coral-400',
    border: 'border-coral-500/30',
  },
  offline: {
    label: 'Offline',
    bg: 'bg-stone-500/20',
    text: 'text-stone-400',
    border: 'border-stone-500/30',
  },
  setup_required: {
    label: 'Setup Required',
    bg: 'bg-primary-500/20',
    text: 'text-primary-400',
    border: 'border-primary-500/30',
  },
};

/**
 * Renders a connection option row with its real-time status badge.
 * Uses useSkillConnectionStatus hook for skill-backed connections.
 */
function ConnectionOptionRow({
  option,
  isFirst,
  isLast,
  onConnect,
}: {
  option: ConnectOption;
  isFirst: boolean;
  isLast: boolean;
  onConnect: (option: ConnectOption) => void;
}) {
  const connectionStatus = useSkillConnectionStatus(option.skillId ?? '');
  const isDisabled = option.comingSoon;

  let badge: React.ReactElement;
  if (option.comingSoon) {
    badge = (
      <span className="px-2 py-1 text-xs font-medium rounded-full border bg-stone-500/20 text-stone-400 border-stone-500/30">
        Coming Soon
      </span>
    );
  } else if (option.skillId) {
    const config = STATUS_BADGE_CONFIG[connectionStatus];
    badge = (
      <span
        className={`px-2 py-1 text-xs font-medium rounded-full border ${config.bg} ${config.text} ${config.border}`}>
        {config.label}
      </span>
    );
  } else {
    badge = (
      <span className="px-2 py-1 text-xs font-medium rounded-full border bg-primary-500/20 text-primary-400 border-primary-500/30">
        Connect
      </span>
    );
  }

  return (
    <button
      onClick={() => onConnect(option)}
      disabled={isDisabled}
      className={`w-full flex items-center justify-between p-3 bg-black/50 ${
        isLast ? '' : 'border-b border-stone-700'
      } hover:bg-stone-800/30 transition-all duration-200 text-left ${
        isFirst ? 'first:rounded-t-3xl' : ''
      } ${
        isLast ? 'last:rounded-b-3xl' : ''
      } focus:outline-none focus:ring-0 focus:border-inherit relative ${
        isDisabled ? 'opacity-60 cursor-not-allowed' : ''
      }`}>
      <div className="w-5 h-5 opacity-60 flex-shrink-0 mr-3 text-white">{option.icon}</div>
      <div className="flex-1">
        <div className="font-medium text-sm mb-1 text-white">{option.name}</div>
        <p className="opacity-70 text-xs">{option.description}</p>
      </div>
      <div className="flex items-center space-x-3">{badge}</div>
    </button>
  );
}

// ---------------------------------------------------------------------------
// Main panel
// ---------------------------------------------------------------------------

const ConnectionsPanel = () => {
  const { navigateBack } = useSettingsNavigation();

  const [setupModalOpen, setSetupModalOpen] = useState(false);
  const [activeSkillId, setActiveSkillId] = useState<string | null>(null);
  const [activeSkillName, setActiveSkillName] = useState<string>('');
  const [activeSkillDescription, setActiveSkillDescription] = useState<string>('');

  const connectOptions: ConnectOption[] = [
    {
      id: 'telegram',
      name: 'Telegram',
      description: 'Organize chats, automate messages and get insights.',
      icon: <img src={TelegramIcon} alt="Telegram" className="w-5 h-5" />,
      skillId: 'telegram',
    },
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
    <div className="overflow-hidden h-full flex flex-col">
      <SettingsHeader title="Connections" showBackButton={true} onBack={navigateBack} />

      <div className="flex-1 overflow-y-auto">
        <div className="p-4 space-y-6">
          {/* Connection Options */}
          <div>
            {connectOptions.map((option, index) => (
              <ConnectionOptionRow
                key={option.id}
                option={option}
                isFirst={index === 0}
                isLast={index === connectOptions.length - 1}
                onConnect={handleConnect}
              />
            ))}
          </div>

          {/* Security notice */}
          <div className="p-4 bg-blue-500/10 border border-blue-500/20 rounded-xl">
            <div className="flex items-start space-x-2">
              <svg
                className="w-5 h-5 text-blue-400 flex-shrink-0 mt-0.5"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                />
              </svg>
              <div>
                <p className="font-medium text-blue-300 text-sm">Privacy & Security</p>
                <p className="text-blue-200 text-xs mt-1">
                  All data and credentials are stored locally with zero-data retention policy. Your
                  information is encrypted and never shared with third parties.
                </p>
              </div>
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
          onClose={() => {
            setSetupModalOpen(false);
            setActiveSkillId(null);
          }}
        />
      )}
    </div>
  );
};

export default ConnectionsPanel;
