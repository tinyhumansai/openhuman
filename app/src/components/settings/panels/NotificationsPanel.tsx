import { useEffect, useState } from 'react';

import { getBypassPrefs, setGlobalDnd } from '../../../services/webviewAccountService';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const NotificationsPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const [dnd, setDnd] = useState(false);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    getBypassPrefs().then(prefs => {
      if (prefs) setDnd(prefs.global_dnd);
      setLoading(false);
    });
  }, []);

  const handleDndToggle = async () => {
    const next = !dnd;
    setDnd(next);
    await setGlobalDnd(next);
  };

  return (
    <div>
      <SettingsHeader
        title="Notifications"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        {loading ? (
          <div className="rounded-xl border border-stone-200 bg-white p-4 text-sm text-stone-400">
            Loading...
          </div>
        ) : (
          <>
            {/* DND toggle */}
            <div>
              <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-400 mb-3 px-1">
                Do Not Disturb
              </h3>
              <div className="bg-white rounded-xl border border-stone-200 overflow-hidden">
                <div className="flex items-center justify-between p-4">
                  <div className="flex-1 mr-4">
                    <p className="text-sm font-medium text-stone-900">Suppress all notifications</p>
                    <p className="text-xs text-stone-500 mt-1 leading-relaxed">
                      Block all OS notification toasts from embedded apps (WhatsApp, Telegram,
                      Gmail, Slack, etc.) regardless of focus state.
                    </p>
                  </div>
                  <button
                    onClick={handleDndToggle}
                    className={`relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
                      dnd ? 'bg-primary-500' : 'bg-stone-600'
                    }`}
                    role="switch"
                    aria-checked={dnd}>
                    <span
                      className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
                        dnd ? 'translate-x-5' : 'translate-x-0'
                      }`}
                    />
                  </button>
                </div>
              </div>
            </div>

            {/* Info box */}
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
                  <p className="text-xs text-stone-500 leading-relaxed">
                    Notifications from embedded apps are also suppressed automatically when you are
                    actively viewing that account in the foreground. This keeps the desktop tidy
                    when you are already reading the conversation the notification would have
                    pointed to.
                  </p>
                </div>
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );
};

export default NotificationsPanel;
