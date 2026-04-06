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

  const groupedMenuItems = [
    {
      id: 'account',
      title: 'Account & Security',
      description: 'Billing, recovery phrase, team management, and linked account access',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 12h14M12 5v14" />
        </svg>
      ),
      onClick: () => navigateToSettings('account'),
      dangerous: false,
    },
    {
      id: 'automation',
      title: 'Automation & Channels',
      description: 'Accessibility, screen intelligence, messaging, autocomplete, and cron jobs',
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
      onClick: () => navigateToSettings('automation'),
      dangerous: false,
    },
    {
      id: 'ai-tools',
      title: 'AI & Skills',
      description: 'Local model runtime, AI configuration, and skills behavior',
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
      onClick: () => navigateToSettings('ai-tools'),
      dangerous: false,
    },
    {
      id: 'developer-options',
      title: 'Developer Options',
      description: 'Diagnostic tools, console access, webhooks, and memory inspection',
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
                  <li>All skills data</li>
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
