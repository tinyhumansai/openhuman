import { type ReactNode, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import LanguageSelect from '../LanguageSelect';
import SettingsHeader from './components/SettingsHeader';
import SettingsMenuItem from './components/SettingsMenuItem';
import { useSettingsNavigation } from './hooks/useSettingsNavigation';
import SettingsSearchBar from './search/SettingsSearchBar';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface SettingsItem {
  id: string;
  title: string;
  description: string;
  icon: ReactNode;
  onClick?: () => void;
  dangerous?: boolean;
  rightElement?: ReactNode;
}

interface SettingsGroup {
  /** Stable identifier for testing and key prop */
  id: string;
  /** i18n label shown above the card */
  label: string;
  items: SettingsItem[];
}

// ---------------------------------------------------------------------------
// Icon helpers (inline SVG kept as constants to avoid duplication)
// ---------------------------------------------------------------------------

const AccountIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z"
    />
  </svg>
);

const LanguageIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M3 5h12M9 3v2m1.048 9.5A18.022 18.022 0 016.412 9m6.088 9h7M11 21l5-10 5 10M12.751 5C11.783 10.77 8.07 15.61 3 18.129"
    />
  </svg>
);

const AppearanceIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M21 12.79A9 9 0 1111.21 3 7 7 0 0021 12.79z"
    />
  </svg>
);

const DevicesIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M12 18h.01M8 21h8a2 2 0 002-2V5a2 2 0 00-2-2H8a2 2 0 00-2 2v14a2 2 0 002 2z"
    />
  </svg>
);

const PersonalityIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z"
    />
  </svg>
);

const MascotIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M12 21a9 9 0 100-18 9 9 0 000 18zM9 10h.01M15 10h.01M9.5 15c.83.67 1.67 1 2.5 1s1.67-.33 2.5-1"
    />
  </svg>
);

const NotificationsIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9"
    />
  </svg>
);

const DeveloperIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4"
    />
  </svg>
);

const AboutIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
    />
  </svg>
);

const DataSyncIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
    />
  </svg>
);

const AiIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z"
    />
  </svg>
);

const AgentsIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z"
    />
  </svg>
);

const FeaturesIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.066 2.573c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.573 1.066c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.066-2.573c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
    />
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
    />
  </svg>
);

const IntegrationsIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M13 10V3L4 14h7v7l9-11h-7z"
    />
  </svg>
);

const CryptoIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M12 8c-1.657 0-3 .895-3 2s1.343 2 3 2 3 .895 3 2-1.343 2-3 2m0-8c1.11 0 2.08.402 2.599 1M12 8V6m0 10c-1.11 0-2.08-.402-2.599-1M12 16v2m0-12a9 9 0 100 18 9 9 0 000-18z"
    />
  </svg>
);

// ---------------------------------------------------------------------------
// Group header (visual separator label above each settings card)
// ---------------------------------------------------------------------------

const GroupHeader = ({ label }: { label: string }) =>
  label ? (
    <div className="px-1 pt-5 pb-1">
      <span className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
        {label}
      </span>
    </div>
  ) : (
    // Empty label → a plain divider (Developer & Diagnostics and About sit
    // after a divider, not under their own section headers).
    <div className="mx-1 mt-6 mb-2 border-t border-stone-200 dark:border-neutral-800" />
  );

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

const SettingsHome = () => {
  const { navigateToSettings } = useSettingsNavigation();
  const { t } = useT();

  // Global settings search. While a query is active the normal menu is hidden
  // and the search bar renders its own ranked result list instead.
  const [searchQuery, setSearchQuery] = useState('');
  const isSearching = searchQuery.trim().length > 0;

  // --- Account group items ---
  // Account (hub), Language (inline), Appearance, Devices, Data Sync.
  const accountItems: SettingsItem[] = [
    {
      id: 'profile',
      title: t('pages.settings.accountSection.title'),
      description: t('pages.settings.accountSection.description'),
      icon: AccountIcon,
      onClick: () => navigateToSettings('account'),
    },
    {
      id: 'language',
      title: t('settings.language'),
      description: t('settings.languageDesc'),
      icon: LanguageIcon,
      rightElement: <LanguageSelect ariaLabel={t('settings.language')} />,
    },
    {
      id: 'appearance',
      title: t('settings.appearance.title'),
      description: t('settings.appearance.menuDesc'),
      icon: AppearanceIcon,
      onClick: () => navigateToSettings('appearance'),
    },
    {
      id: 'devices',
      title: t('settings.account.devices'),
      description: t('settings.account.devicesDesc'),
      icon: DevicesIcon,
      onClick: () => navigateToSettings('devices'),
    },
    {
      id: 'data-sync',
      title: t('settings.dataSync.title'),
      description: t('settings.dataSync.menuDesc'),
      icon: DataSyncIcon,
      onClick: () => navigateToSettings('memory-sync'),
    },
  ];

  // --- Assistant group items ---
  // AI & Models, Agents, Personality, Face & Mascot.
  const assistantItems: SettingsItem[] = [
    {
      id: 'ai',
      title: t('pages.settings.aiSection.title'),
      description: t('pages.settings.aiSection.description'),
      icon: AiIcon,
      onClick: () => navigateToSettings('ai'),
    },
    {
      id: 'agents-settings',
      title: t('settings.agentsSection.title'),
      description: t('settings.agentsSection.menuDesc'),
      icon: AgentsIcon,
      onClick: () => navigateToSettings('agents-settings'),
    },
    {
      id: 'persona',
      title: t('settings.assistant.personality'),
      description: t('settings.assistant.personalityDesc'),
      icon: PersonalityIcon,
      onClick: () => navigateToSettings('persona'),
    },
    {
      id: 'mascot',
      title: t('settings.assistant.faceMascot'),
      description: t('settings.assistant.faceMascotDesc'),
      icon: MascotIcon,
      onClick: () => navigateToSettings('mascot'),
    },
  ];

  // --- Features & Integrations group items ---
  // Features section, Composio/Integrations section.
  const featuresIntegrationsItems: SettingsItem[] = [
    {
      id: 'features',
      title: t('pages.settings.featuresSection.title'),
      description: t('pages.settings.featuresSection.description'),
      icon: FeaturesIcon,
      onClick: () => navigateToSettings('features'),
    },
    {
      id: 'composio',
      title: t('pages.settings.composioSection.title'),
      description: t('pages.settings.composioSection.description'),
      icon: IntegrationsIcon,
      onClick: () => navigateToSettings('composio'),
    },
  ];

  // --- Notifications group ---
  const notificationsItems: SettingsItem[] = [
    {
      id: 'notifications-hub',
      title: t('settings.notifications.menuTitle'),
      description: t('settings.notifications.menuDesc'),
      icon: NotificationsIcon,
      onClick: () => navigateToSettings('notifications-hub'),
    },
  ];

  // --- Crypto group ---
  const cryptoItems: SettingsItem[] = [
    {
      id: 'crypto',
      title: t('settings.cryptoSection.title'),
      description: t('settings.cryptoSection.menuDesc'),
      icon: CryptoIcon,
      onClick: () => navigateToSettings('crypto'),
    },
  ];

  // The layman-facing merged card combines: Account, Assistant,
  // Features & Integrations, Notifications, Crypto rows in one flat card.
  const laymanItems: SettingsItem[] = [
    ...accountItems,
    ...assistantItems,
    ...featuresIntegrationsItems,
    ...notificationsItems,
    ...cryptoItems,
  ];

  // --- About group (always visible; no section header — just a divider) ---
  const aboutGroup: SettingsGroup = {
    id: 'about',
    label: '',
    items: [
      {
        id: 'about',
        title: t('settings.about'),
        description: t('settings.aboutDesc'),
        icon: AboutIcon,
        onClick: () => navigateToSettings('about'),
      },
    ],
  };

  // --- Developer & Diagnostics (always visible) ---
  const developerGroup: SettingsGroup = {
    id: 'developer',
    label: '',
    items: [
      {
        id: 'developer-options',
        title: t('settings.developerDiagnostics'),
        description: t('settings.developerDiagnosticsDesc'),
        icon: DeveloperIcon,
        onClick: () => navigateToSettings('developer-options'),
      },
    ],
  };

  const trailingGroups: SettingsGroup[] = [developerGroup, aboutGroup];

  return (
    <div className="z-10 relative">
      <div data-walkthrough="settings-menu">
        <SettingsHeader />
      </div>

      <SettingsSearchBar value={searchQuery} onValueChange={setSearchQuery} />

      {/* While searching, the search bar renders its own results and the normal
          settings menu is hidden to avoid a confusing double list. */}
      {isSearching ? null : (
        <div className="px-4 pt-3 pb-5">
          {/* Merged layman card — Account / Assistant / Features & Integrations /
              Notifications / Crypto in one flat card. No sub-section headers. */}
          <div
            data-testid="settings-group-main"
            className="rounded-3xl overflow-hidden border border-stone-200 dark:border-neutral-800">
            {laymanItems.map((item, index) => (
              <SettingsMenuItem
                key={item.id}
                icon={item.icon}
                title={item.title}
                description={item.description}
                onClick={item.onClick}
                testId={`settings-nav-${item.id}`}
                dangerous={item.dangerous}
                isFirst={index === 0}
                isLast={index === laymanItems.length - 1}
                rightElement={item.rightElement}
              />
            ))}
          </div>

          {trailingGroups.map(group => (
            <div key={group.id} data-testid={`settings-group-${group.id}`}>
              <GroupHeader label={group.label} />
              <div className="rounded-3xl overflow-hidden border border-stone-200 dark:border-neutral-800">
                {group.items.map((item, index) => (
                  <SettingsMenuItem
                    key={item.id}
                    icon={item.icon}
                    title={item.title}
                    description={item.description}
                    onClick={item.onClick}
                    testId={`settings-nav-${item.id}`}
                    dangerous={item.dangerous}
                    isFirst={index === 0}
                    isLast={index === group.items.length - 1}
                    rightElement={item.rightElement}
                  />
                ))}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};

export default SettingsHome;
