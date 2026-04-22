import type { ReactElement } from 'react';

import BinanceIcon from '../../../assets/icons/binance.svg';
import GoogleIcon from '../../../assets/icons/GoogleIcon';
import MetamaskIcon from '../../../assets/icons/metamask.svg';
import NotionIcon from '../../../assets/icons/notion.svg';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

interface ConnectOption {
  id: string;
  name: string;
  description: string;
  icon: ReactElement;
  comingSoon?: boolean;
  skillId?: string;
}

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
  const isDisabled = option.comingSoon;

  const badge = option.comingSoon ? (
    <span className="px-2 py-0.5 text-[11px] font-medium rounded-full bg-stone-100 text-stone-500 border border-stone-200">
      Coming soon
    </span>
  ) : (
    <span className="px-2 py-0.5 text-[11px] font-medium rounded-full bg-primary-50 text-primary-600 border border-primary-100">
      Connect
    </span>
  );

  return (
    <button
      onClick={() => onConnect(option)}
      disabled={isDisabled}
      className={`group w-full flex items-center justify-between p-4 bg-white text-left transition-colors duration-150 ${
        isLast ? '' : 'border-b border-stone-200'
      } ${isFirst ? 'rounded-t-2xl' : ''} ${isLast ? 'rounded-b-2xl' : ''} ${
        isDisabled ? 'opacity-70 cursor-not-allowed' : 'hover:bg-stone-50 focus-visible:bg-stone-50'
      } focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-500/40`}>
      <div
        className={`w-5 h-5 flex-shrink-0 mr-3 text-stone-700 ${
          isDisabled ? 'opacity-50' : 'opacity-80 group-hover:opacity-100'
        } transition-opacity`}>
        {option.icon}
      </div>
      <div className="flex-1 min-w-0">
        <div className="font-medium text-sm text-stone-900 leading-snug">{option.name}</div>
        <p className="text-xs text-stone-500 mt-0.5 leading-relaxed truncate">
          {option.description}
        </p>
      </div>
      <div className="flex items-center ml-3">{badge}</div>
    </button>
  );
}

// ---------------------------------------------------------------------------
// Main panel
// ---------------------------------------------------------------------------

const ConnectionsPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

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
    // TODO(connections): replace these early returns with the real connect flow.
    // When `option.comingSoon` is false or `option.skillId` is wired, this should
    // open the connection modal, route, or dispatch the connect action.
    if (option.comingSoon) return;
    if (option.skillId) return;
  };

  return (
    <div>
      <SettingsHeader
        title="Connections"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div>
        <div className="p-4 space-y-4">
          {/* Connection Options */}
          <div className="rounded-2xl border border-stone-200 overflow-hidden bg-white">
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

          {/* Security notice — palette aligned with Privacy & Security panel for cross-surface trust coherence */}
          <div className="p-4 bg-stone-50 rounded-xl border border-stone-200">
            <div className="flex items-start space-x-3">
              <svg
                className="w-5 h-5 text-stone-400 mt-0.5 flex-shrink-0"
                fill="currentColor"
                viewBox="0 0 20 20">
                <path
                  fillRule="evenodd"
                  d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7-4a1 1 0 11-2 0 1 1 0 012 0zM9 9a1 1 0 000 2v3a1 1 0 001 1h1a1 1 0 100-2v-3a1 1 0 00-1-1H9z"
                  clipRule="evenodd"
                />
              </svg>
              <div>
                <p className="font-medium text-stone-900 text-sm">Privacy & Security</p>
                <p className="text-xs text-stone-500 mt-1 leading-relaxed">
                  All data and credentials are stored locally with zero-data retention policy. Your
                  information is encrypted and never shared with third parties.
                </p>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default ConnectionsPanel;
