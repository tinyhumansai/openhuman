import { type ReactElement, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import BinanceIcon from '../../../assets/icons/binance.svg';
import GoogleIcon from '../../../assets/icons/GoogleIcon';
import MetamaskIcon from '../../../assets/icons/metamask.svg';
import NotionIcon from '../../../assets/icons/notion.svg';
import { useT } from '../../../lib/i18n/I18nContext';
import { fetchWalletStatus, type WalletStatus } from '../../../services/walletApi';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

interface ConnectOption {
  id: string;
  name: string;
  description: string;
  icon: ReactElement;
  comingSoon?: boolean;
  statusLabel?: string;
  skillId?: string;
}

function ConnectionOptionRow({
  option,
  isFirst,
  isLast,
  onConnect,
  t,
}: {
  option: ConnectOption;
  isFirst: boolean;
  isLast: boolean;
  onConnect: (option: ConnectOption) => void;
  t: (key: string) => string;
}) {
  const isDisabled = option.comingSoon;

  const badge = option.comingSoon ? (
    <span className="px-2 py-0.5 text-[11px] font-medium rounded-full bg-stone-100 text-stone-500 border border-stone-200">
      {t('connections.comingSoon')}
    </span>
  ) : option.statusLabel ? (
    <span className="px-2 py-0.5 text-[11px] font-medium rounded-full bg-sage-50 text-sage-700 border border-sage-200">
      {option.statusLabel}
    </span>
  ) : (
    <span className="px-2 py-0.5 text-[11px] font-medium rounded-full bg-primary-50 text-primary-600 border border-primary-100">
      {t('connections.setUp')}
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

const ConnectionsPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const navigate = useNavigate();
  const [walletStatus, setWalletStatus] = useState<WalletStatus | null>(null);
  const [walletStatusState, setWalletStatusState] = useState<'loading' | 'ready' | 'error'>(
    'loading'
  );

  useEffect(() => {
    let active = true;
    fetchWalletStatus()
      .then(status => {
        if (active) {
          setWalletStatus(status);
          setWalletStatusState('ready');
        }
      })
      .catch(() => {
        if (active) {
          setWalletStatusState('error');
        }
      });
    return () => {
      active = false;
    };
  }, []);

  const walletReady = walletStatusState === 'ready';
  const walletConfigured = walletReady && walletStatus?.configured === true;

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
      description: walletConfigured
        ? t('connections.walletConfigured')
        : walletReady
          ? t('connections.walletReady')
          : walletStatusState === 'error'
            ? t('connections.walletError')
            : t('connections.walletChecking'),
      icon: <img src={MetamaskIcon} alt="Metamask" className="w-5 h-5" />,
      statusLabel: walletConfigured
        ? t('connections.configured')
        : walletReady
          ? undefined
          : walletStatusState === 'error'
            ? t('connections.unavailable')
            : t('connections.checking'),
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
    if (option.id === 'wallet') {
      navigate('/settings/recovery-phrase');
      return;
    }
    if (option.skillId) return;
  };

  return (
    <div>
      <SettingsHeader
        title={t('settings.account.connections')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div>
        <div className="p-4 space-y-4">
          <div className="rounded-2xl border border-stone-200 overflow-hidden bg-white">
            {connectOptions.map((option, index) => (
              <ConnectionOptionRow
                key={option.id}
                option={option}
                isFirst={index === 0}
                isLast={index === connectOptions.length - 1}
                onConnect={handleConnect}
                t={t}
              />
            ))}
          </div>

          {walletConfigured && walletStatus ? (
            <div className="rounded-2xl border border-stone-200 bg-white p-4 space-y-3">
              <div>
                <p className="font-medium text-stone-900 text-sm">
                  {t('connections.walletIdentities')}
                </p>
                <p className="text-xs text-stone-500 mt-1">{t('connections.walletDerived')}</p>
              </div>
              <div className="grid gap-2">
                {walletStatus.accounts.map(account => (
                  <div
                    key={account.chain}
                    className="rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
                    <div className="flex items-center justify-between gap-3">
                      <span className="text-xs font-semibold uppercase tracking-wide text-stone-500">
                        {account.chain}
                      </span>
                      <span className="text-xs font-mono text-stone-700 truncate">
                        {account.address}
                      </span>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          ) : null}

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
                <p className="font-medium text-stone-900 text-sm">
                  {t('connections.privacySecurity')}
                </p>
                <p className="text-xs text-stone-500 mt-1 leading-relaxed">
                  {t('connections.privacySecurityDesc')}
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
