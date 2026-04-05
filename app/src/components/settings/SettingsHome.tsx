import { useState } from 'react';

import { skillManager } from '../../lib/skills/manager';
import { useCoreState } from '../../providers/CoreStateProvider';
import { persistor } from '../../store';
import { resetOpenHumanDataAndRestartCore } from '../../utils/tauriCommands';
import SettingsHeader from './components/SettingsHeader';
import SettingsMenuItem from './components/SettingsMenuItem';
import { useSettingsNavigation } from './hooks/useSettingsNavigation';

const SettingsHome = () => {
  const { navigateToSettings } = useSettingsNavigation();
  const { clearSession, setOnboardingCompletedFlag } = useCoreState();
  const [showLogoutAndClearModal, setShowLogoutAndClearModal] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleLogout = async () => {
    try {
      await setOnboardingCompletedFlag(false);
    } catch (err) {
      console.warn('[Settings] Failed to clear onboarding_completed in config:', err);
    }
    try {
      await clearSession();
    } catch (err) {
      console.warn('[Settings] Rust logout failed:', err);
    }
    window.location.hash = '/';
  };

  const clearAllAppData = async () => {
    try {
      await clearSession();
    } catch (err) {
      console.warn('[Settings] Rust logout failed during clearAllAppData:', err);
    }

    try {
      await resetOpenHumanDataAndRestartCore();
    } catch (err) {
      console.warn('[Settings] Failed to reset OpenHuman data dir and restart core:', err);
      throw err;
    }

    // Best-effort cleanup for in-memory and browser-side caches that live outside the Rust core.
    try {
      await skillManager.clearAllSkillsData();
    } catch (error) {
      console.warn('Failed to clear skills data:', error);
      // Continue even if skill cleanup fails because the backend reset already completed.
    }

    await persistor.purge();
    window.localStorage.clear();
    window.sessionStorage.clear();

    // Complete reset - redirect to login for fresh start
    window.location.hash = '/';
  };

  const handleLogoutAndClearData = async () => {
    try {
      setIsLoading(true);
      setError(null);
      await clearAllAppData(); // This will redirect to login
    } catch (_error) {
      setError('Failed to clear data and logout. Please try again.');
      setIsLoading(false);
    }
  };

  // const handleViewEncryptionKey = () => {
  //   // TODO: Show encryption key in a secure modal
  //   console.log('View encryption key');
  // };

  const handleDeleteAllData = () => {
    // TODO: Show confirmation dialog and delete all data
    console.log('Delete all data');
  };

  // Main settings menu items
  const mainMenuItems = [
    {
      id: 'accessibility',
      title: 'Accessibility Automation',
      description: 'Desktop permissions, assisted controls, and safety-bound sessions',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M9 12h6m-7 9h8a2 2 0 002-2V7a2 2 0 00-2-2h-1l-.707-.707A1 1 0 0013.586 4h-3.172a1 1 0 00-.707.293L9 5H8a2 2 0 00-2 2v12a2 2 0 002 2z"
          />
        </svg>
      ),
      onClick: () => navigateToSettings('accessibility'),
      dangerous: false,
    },
    {
      id: 'screen-intelligence',
      title: 'Screen Intelligence',
      description: 'Window capture policy, vision summaries, and memory ingestion',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M3 5h18v12H3zM8 21h8m-4-4v4"
          />
        </svg>
      ),
      onClick: () => navigateToSettings('screen-intelligence'),
      dangerous: false,
    },
    {
      id: 'autocomplete',
      title: 'Inline Autocomplete',
      description: 'Manage predictive text style, app filters, and live completion controls',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M4 7h16M4 12h10m-10 5h7m10 0l3 3m0 0l3-3m-3 3v-8"
          />
        </svg>
      ),
      onClick: () => navigateToSettings('autocomplete'),
      dangerous: false,
    },
    {
      id: 'messaging',
      title: 'Messaging Channels',
      description: 'Configure Telegram/Discord auth modes and default channel routing',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M8 10h.01M12 10h.01M16 10h.01M21 11c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 19l1.395-3.72C3.512 14.042 3 12.574 3 11c0-4.418 4.03-8 9-8s9 3.582 9 8z"
          />
        </svg>
      ),
      onClick: () => navigateToSettings('messaging'),
      dangerous: false,
    },
    {
      id: 'cron-jobs',
      title: 'Cron Jobs',
      description: 'View and configure available scheduled jobs for runtime skills',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
          />
        </svg>
      ),
      onClick: () => navigateToSettings('cron-jobs'),
      dangerous: false,
    },
    // {
    //   id: "messaging",
    //   title: "Messaging",
    //   description: "Configure messaging preferences and templates",
    //   icon: (
    //     <svg
    //       className="w-5 h-5"
    //       fill="none"
    //       stroke="currentColor"
    //       viewBox="0 0 24 24"
    //     >
    //       <path
    //         strokeLinecap="round"
    //         strokeLinejoin="round"
    //         strokeWidth={2}
    //         d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"
    //       />
    //     </svg>
    //   ),
    //   onClick: () => navigateToSettings("messaging"),
    //   dangerous: false,
    // },
    // {
    //   id: 'agent-chat',
    //   title: 'Agent Chat',
    //   description: 'Send messages directly to your agent',
    //   icon: (
    //     <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    //       <path
    //         strokeLinecap="round"
    //         strokeLinejoin="round"
    //         strokeWidth={2}
    //         d="M8 10h8m-8 4h5m-6 6l-4 4V6a2 2 0 012-2h12a2 2 0 012 2v9a2 2 0 01-2 2H7z"
    //       />
    //     </svg>
    //   ),
    //   onClick: () => navigateToSettings('agent-chat'),
    //   dangerous: false,
    // },
    // {
    //   id: 'privacy',
    //   title: 'Privacy & Security',
    //   description: 'Control your privacy and security settings',
    //   icon: (
    //     <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    //       <path
    //         strokeLinecap="round"
    //         strokeLinejoin="round"
    //         strokeWidth={2}
    //         d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z"
    //       />
    //     </svg>
    //   ),
    //   onClick: () => navigateToSettings('privacy'),
    //   dangerous: false,
    // },
    // {
    //   id: 'profile',
    //   title: 'Profile',
    //   description: 'Update your profile information and preferences',
    //   icon: (
    //     <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    //       <path
    //         strokeLinecap="round"
    //         strokeLinejoin="round"
    //         strokeWidth={2}
    //         d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z"
    //       />
    //     </svg>
    //   ),
    //   onClick: () => navigateToSettings('profile'),
    //   dangerous: false,
    // },
    // {
    //   id: 'advanced',
    //   title: 'Advanced',
    //   description: 'Advanced configuration and developer options',
    //   icon: (
    //     <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    //       <path
    //         strokeLinecap="round"
    //         strokeLinejoin="round"
    //         strokeWidth={2}
    //         d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
    //       />
    //       <path
    //         strokeLinecap="round"
    //         strokeLinejoin="round"
    //         strokeWidth={2}
    //         d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
    //       />
    //     </svg>
    //   ),
    //   onClick: () => navigateToSettings('advanced'),
    //   dangerous: false,
    // },
    // {
    //   id: 'encryption',
    //   title: 'View Encryption Key',
    //   description: 'Access your encryption key for backup purposes',
    //   icon: (
    //     <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    //       <path
    //         strokeLinecap="round"
    //         strokeLinejoin="round"
    //         strokeWidth={2}
    //         d="M15 7a2 2 0 0 1 2 2m4 0a6 6 0 0 1-7.743 5.743L11 17H9v2H7v2H4a1 1 0 0 1-1-1v-2.586a1 1 0 0 1 .293-.707l5.964-5.964A6 6 0 1 1 21 9z"
    //       />
    //     </svg>
    //   ),
    //   onClick: handleViewEncryptionKey,
    //   dangerous: false,
    // },
    {
      id: 'local-model',
      title: 'Local AI Model',
      description: 'Choose model tier by device capability and manage downloads',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z"
          />
        </svg>
      ),
      onClick: () => navigateToSettings('local-model'),
      dangerous: false,
    },
    {
      id: 'team',
      title: 'Team',
      description: 'Manage your team and invite members',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0zm6 3a2 2 0 11-4 0 2 2 0 014 0zM7 10a2 2 0 11-4 0 2 2 0 014 0z"
          />
        </svg>
      ),
      onClick: () => navigateToSettings('team'),
      dangerous: false,
    },
    {
      id: 'billing',
      title: 'Billing & Usage',
      description: 'Manage your subscription and payment methods',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M3 10h18M7 15h1m4 0h1m-7 4h12a3 3 0 003-3V8a3 3 0 00-3 3v8a3 3 0 003 3z"
          />
        </svg>
      ),
      onClick: () => navigateToSettings('billing'),
      dangerous: false,
    },
    {
      id: 'recovery-phrase',
      title: 'Recovery Phrase',
      description: 'Generate or import your BIP39 recovery phrase for encryption and wallet',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z"
          />
        </svg>
      ),
      onClick: () => navigateToSettings('recovery-phrase'),
      dangerous: false,
    },
    {
      id: 'developer-options',
      title: 'Developer Options',
      description: 'Skills, AI config, Tauri console, and memory debug tools',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4"
          />
        </svg>
      ),
      onClick: () => navigateToSettings('developer-options'),
      dangerous: false,
    },
  ];

  // Destructive actions menu items
  const destructiveMenuItems = [
    {
      id: 'delete',
      title: 'Delete All Data',
      description: 'Permanently delete all your data and reset your account',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
          />
        </svg>
      ),
      onClick: handleDeleteAllData,
      dangerous: true,
    },
    {
      id: 'logout-and-clear',
      title: 'Log Out & Clear App Data',
      description: 'Sign out and permanently clear all local data',
      icon: (
        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1"
          />
        </svg>
      ),
      onClick: () => setShowLogoutAndClearModal(true),
      dangerous: true,
    },
    {
      id: 'logout',
      title: 'Log out',
      description: 'Sign out of your account',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1"
          />
        </svg>
      ),
      onClick: handleLogout,
      dangerous: true,
    },
  ];

  return (
    <div className="overflow-hidden h-full flex flex-col z-10 relative">
      <SettingsHeader />

      <div className="flex-1 overflow-y-auto">
        <div className="p-4 space-y-6">
          {/* Main Settings */}
          <div>
            {mainMenuItems.map((item, index) => (
              <SettingsMenuItem
                key={item.id}
                icon={item.icon}
                title={item.title}
                description={item.description}
                onClick={item.onClick}
                dangerous={item.dangerous}
                isFirst={index === 0}
                isLast={index === mainMenuItems.length - 1}
              />
            ))}
          </div>

          {/* Destructive Actions */}
          <div>
            {destructiveMenuItems.map((item, index) => (
              <SettingsMenuItem
                key={item.id}
                icon={item.icon}
                title={item.title}
                description={item.description}
                onClick={item.onClick}
                dangerous={item.dangerous}
                isFirst={index === 0}
                isLast={index === destructiveMenuItems.length - 1}
              />
            ))}
          </div>
        </div>
      </div>

      {/* Log Out & Clear Data Confirmation Modal */}
      {showLogoutAndClearModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/30">
          <div className="bg-white rounded-2xl max-w-md w-full p-6 border border-stone-200">
            <div className="flex items-center gap-3 mb-4">
              <div className="w-10 h-10 rounded-lg bg-amber-100 flex items-center justify-center">
                <svg
                  className="w-5 h-5 text-amber-400"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1"
                  />
                </svg>
              </div>
              <div>
                <h3 className="text-lg font-semibold text-stone-900">Log Out & Clear App Data</h3>
              </div>
            </div>

            <div className="mb-6">
              <p className="text-stone-700 text-sm leading-relaxed">
                This will sign you out and permanently delete ALL data including: • App settings and
                conversations • Email data from Gmail • Chat history from Telegram • Cached files
                from Notion • All other skills data
                <br />
                <br />
                This action cannot be undone and may take a few moments to complete.
              </p>

              {error && (
                <div className="mt-3 p-3 rounded-lg bg-coral-100 border border-coral-500/20">
                  <p className="text-coral-600 text-sm">{error}</p>
                </div>
              )}
            </div>

            <div className="flex gap-3">
              <button
                onClick={() => {
                  setShowLogoutAndClearModal(false);
                  setError(null);
                }}
                disabled={isLoading}
                className="flex-1 px-4 py-2 rounded-lg border border-stone-200 text-stone-700 hover:bg-stone-100 transition-colors disabled:opacity-50">
                Cancel
              </button>
              <button
                onClick={handleLogoutAndClearData}
                disabled={isLoading}
                className="flex-1 px-4 py-2 rounded-lg bg-amber-600 hover:bg-amber-500 text-white transition-colors disabled:opacity-50 flex items-center justify-center gap-2">
                {isLoading && (
                  <svg className="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
                    <circle
                      className="opacity-25"
                      cx="12"
                      cy="12"
                      r="10"
                      stroke="currentColor"
                      strokeWidth="4"
                    />
                    <path
                      className="opacity-75"
                      fill="currentColor"
                      d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                    />
                  </svg>
                )}
                {isLoading ? 'Clearing All Data...' : 'Log Out & Clear Everything'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

export default SettingsHome;
