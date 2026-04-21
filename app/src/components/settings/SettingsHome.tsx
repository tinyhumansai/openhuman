import { useState } from 'react';

import { useCoreState } from '../../providers/CoreStateProvider';
import { persistor } from '../../store';
import { resetOpenHumanDataAndRestartCore } from '../../utils/tauriCommands';
import SettingsHeader from './components/SettingsHeader';
import SettingsMenuItem from './components/SettingsMenuItem';
import { useSettingsNavigation } from './hooks/useSettingsNavigation';

const SettingsHome = () => {
  const { navigateToSettings } = useSettingsNavigation();
  const { clearSession } = useCoreState();
  const [showLogoutAndClearModal, setShowLogoutAndClearModal] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleLogout = async () => {
    try {
      await clearSession();
    } catch (err) {
      console.warn('[Settings] Rust logout failed:', err);
      setError('Failed to log out. Please try again.');
    }
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

    await persistor.purge();
    window.localStorage.clear();
    window.sessionStorage.clear();
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

  const groupedMenuItems = [
    {
      id: 'account',
      title: 'Account',
      description: 'Recovery phrase, team, connections, and privacy',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z"
          />
        </svg>
      ),
      onClick: () => navigateToSettings('account'),
      dangerous: false,
    },
    {
      id: 'billing',
      title: 'Billing & Usage',
      description: 'Subscription plan, pay-as-you-go credits, and payment methods',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M3 10h18M7 15h1m4 0h1m-7 4h12a3 3 0 003-3V8a3 3 0 00-3-3H5a3 3 0 00-3 3v8a3 3 0 003 3z"
          />
        </svg>
      ),
      onClick: () => navigateToSettings('billing'),
      dangerous: false,
    },
    {
      id: 'features',
      title: 'Features',
      description: 'Screen awareness, autocomplete, voice, messaging, and tools',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M13 10V3L4 14h7v7l9-11h-7z"
          />
        </svg>
      ),
      onClick: () => navigateToSettings('features'),
      dangerous: false,
    },
    {
      id: 'ai-models',
      title: 'AI & Models',
      description: 'Local AI model setup and downloads',
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
      onClick: () => navigateToSettings('ai-models'),
      dangerous: false,
    },
    {
      id: 'notifications',
      title: 'Notifications',
      description: 'Do Not Disturb and per-account notification controls',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9"
          />
        </svg>
      ),
      onClick: () => navigateToSettings('notifications'),
      dangerous: false,
    },
    {
      id: 'notification-routing',
      title: 'Notification Routing',
      description: 'AI importance scoring and orchestrator escalation for integration alerts',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z"
          />
        </svg>
      ),
      onClick: () => navigateToSettings('notification-routing'),
      dangerous: false,
    },
    {
      id: 'developer-options',
      title: 'Developer Options',
      description: 'Diagnostics, debug panels, webhooks, and memory inspection',
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
      id: 'logout-and-clear',
      title: 'Clear App Data',
      description: 'Sign out and permanently clear all local app data',
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
    <div className="z-10 relative">
      <SettingsHeader />

      <div>
        {/* Grouped Settings */}
        {groupedMenuItems.map((item, index) => (
          <SettingsMenuItem
            key={item.id}
            icon={item.icon}
            title={item.title}
            description={item.description}
            onClick={item.onClick}
            dangerous={item.dangerous}
            isFirst={index === 0}
            // isLast={index === groupedMenuItems.length - 1}
          />
        ))}

        {/* Destructive Actions */}
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
                <h3 className="text-lg font-semibold text-stone-900">Clear App Data</h3>
              </div>
            </div>

            <div className="mb-6">
              <div className="text-stone-700 text-sm leading-relaxed">
                <p>This will sign you out and permanently delete local app data including:</p>
                <ul className="list-disc pl-5 mt-2 space-y-1">
                  <li>App settings and conversations</li>
                  <li>All local integration cache data</li>
                  <li>Workspace data</li>
                  <li>All other local data</li>
                </ul>
                <p className="mt-3">This action cannot be undone.</p>
              </div>

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
                className="flex-1 px-4 py-2 rounded-sm bg-amber-600 hover:bg-amber-500 text-white transition-colors disabled:opacity-50 flex items-center justify-center gap-2">
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
                {isLoading ? 'Clearing App Data...' : 'Clear App Data'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

export default SettingsHome;
