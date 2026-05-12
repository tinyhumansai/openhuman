import { useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { getBypassPrefs, setGlobalDnd } from '../../../services/webviewAccountService';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { type NotificationCategory, setPreference } from '../../../store/notificationSlice';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const CATEGORIES: { id: NotificationCategory; title: string; description: string }[] = [
  {
    id: 'messages',
    title: 'Messages',
    description: 'New messages from embedded webview accounts (Slack, WhatsApp, …).',
  },
  {
    id: 'agents',
    title: 'Agent activity',
    description: 'Agent task completions and long-running responses.',
  },
  { id: 'skills', title: 'Skills', description: 'Skill sync events and OAuth status changes.' },
  {
    id: 'system',
    title: 'System',
    description: 'Connection issues, background process errors, updates.',
  },
  {
    id: 'meetings',
    title: 'Meetings',
    description: 'Upcoming meetings and calendar events detected by heartbeat.',
  },
  {
    id: 'reminders',
    title: 'Reminders',
    description: 'Upcoming reminders and scheduled tasks from cron jobs.',
  },
  {
    id: 'important',
    title: 'Important events',
    description: 'Urgent or time-sensitive events surfaced from connected sources.',
  },
];

const NotificationsPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const preferences = useAppSelector(s => s.notifications.preferences);
  const dispatch = useAppDispatch();
  const [dnd, setDnd] = useState(false);
  const [dndLoading, setDndLoading] = useState(true);
  const [dndSaving, setDndSaving] = useState(false);

  useEffect(() => {
    getBypassPrefs().then(prefs => {
      if (prefs) setDnd(prefs.global_dnd);
      setDndLoading(false);
    });
  }, []);

  const handleToggle = (category: NotificationCategory) => {
    dispatch(setPreference({ category, enabled: !preferences[category] }));
  };

  const handleDndToggle = async () => {
    if (dndSaving) return; // prevent concurrent writes
    const next = !dnd;
    setDnd(next);
    setDndSaving(true);
    try {
      await setGlobalDnd(next);
    } catch {
      // Roll back optimistic UI update on failure.
      setDnd(!next);
    } finally {
      setDndSaving(false);
    }
  };

  return (
    <div>
      <SettingsHeader
        title={t('settings.notifications')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div>
        <div className="p-4 space-y-4">
          {/* Do Not Disturb */}
          <div>
            <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-400 mb-2 px-1">
              {t('settings.notifications.doNotDisturb')}
            </h3>
            <div className="bg-white rounded-xl border border-stone-200 overflow-hidden">
              <div className="flex items-center justify-between p-4">
                <div className="flex-1 mr-4">
                  <p className="text-sm font-medium text-stone-900">
                    {t('settings.notifications.suppressAll')}
                  </p>
                  <p className="text-xs text-stone-500 mt-1 leading-relaxed">
                    {t('settings.notifications.suppressAllDesc')}
                  </p>
                </div>
                {dndLoading ? (
                  <div className="w-11 h-6 rounded-full bg-stone-200 animate-pulse" />
                ) : (
                  <button
                    onClick={() => {
                      void handleDndToggle();
                    }}
                    disabled={dndSaving}
                    className={`relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-500 focus-visible:ring-offset-1 disabled:opacity-70 ${
                      dnd ? 'bg-primary-500' : 'bg-stone-400'
                    }`}
                    role="switch"
                    aria-checked={dnd}
                    aria-label={t('settings.notifications.toggleDnd')}>
                    <span
                      className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
                        dnd ? 'translate-x-5' : 'translate-x-0'
                      }`}
                    />
                  </button>
                )}
              </div>
            </div>
          </div>

          {/* Categories */}
          <div>
            <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-400 mb-2 px-1">
              {t('settings.notifications.categories')}
            </h3>
            <div className="bg-white rounded-xl border border-stone-200 overflow-hidden divide-y divide-stone-100">
              {CATEGORIES.map(cat => {
                const enabled = preferences[cat.id];
                return (
                  <div key={cat.id} className="flex items-center justify-between p-4">
                    <div className="flex-1 mr-4">
                      <p className="text-sm font-medium text-stone-900">{cat.title}</p>
                      <p className="text-xs text-stone-500 mt-1 leading-relaxed">
                        {cat.description}
                      </p>
                    </div>
                    <button
                      onClick={() => handleToggle(cat.id)}
                      className={`relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-500 focus-visible:ring-offset-1 ${
                        enabled ? 'bg-primary-500' : 'bg-stone-400'
                      }`}
                      role="switch"
                      aria-checked={enabled}
                      aria-label={`Toggle ${cat.title} notifications`}>
                      <span
                        className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
                          enabled ? 'translate-x-5' : 'translate-x-0'
                        }`}
                      />
                    </button>
                  </div>
                );
              })}
            </div>

            <p className="text-xs text-stone-500 leading-relaxed px-1 mt-2">
              {t('settings.notifications.categoryFooter')}
            </p>
          </div>
        </div>
      </div>
    </div>
  );
};

export default NotificationsPanel;
