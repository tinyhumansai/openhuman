import { useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { getBypassPrefs, setGlobalDnd } from '../../../services/webviewAccountService';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { type NotificationCategory, setPreference } from '../../../store/notificationSlice';
import SettingsHeader from '../components/SettingsHeader';
import { SettingsRow, SettingsSection, SettingsSwitch } from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

interface NotificationsPanelProps {
  /** When embedded inside the tabbed Notifications page, the parent owns the
      `<SettingsHeader>` chrome and we render only the body. */
  embedded?: boolean;
}

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

const NotificationsPanel = ({ embedded = false }: NotificationsPanelProps = {}) => {
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
      {!embedded && (
        <SettingsHeader
          title={t('settings.notifications')}
          showBackButton={true}
          onBack={navigateBack}
          breadcrumbs={breadcrumbs}
        />
      )}

      <div className="p-4 space-y-4">
        {/* Do Not Disturb */}
        <SettingsSection title={t('settings.notifications.doNotDisturb')}>
          <SettingsRow
            htmlFor="switch-dnd"
            label={t('settings.notifications.suppressAll')}
            description={t('settings.notifications.suppressAllDesc')}
            control={
              dndLoading ? (
                <div className="w-[38px] h-[22px] rounded-full bg-neutral-200 dark:bg-neutral-800 animate-pulse" />
              ) : (
                <SettingsSwitch
                  id="switch-dnd"
                  checked={dnd}
                  onCheckedChange={() => {
                    void handleDndToggle();
                  }}
                  disabled={dndSaving}
                  aria-label={t('settings.notifications.toggleDnd')}
                />
              )
            }
          />
        </SettingsSection>

        {/* Categories */}
        <SettingsSection title={t('settings.notifications.categories')}>
          {CATEGORIES.map(cat => {
            const enabled = preferences[cat.id];
            const switchId = `switch-notif-${cat.id}`;
            return (
              <SettingsRow
                key={cat.id}
                htmlFor={switchId}
                label={cat.title}
                description={cat.description}
                control={
                  <SettingsSwitch
                    id={switchId}
                    checked={enabled}
                    onCheckedChange={() => handleToggle(cat.id)}
                    aria-label={`Toggle ${cat.title} notifications`}
                  />
                }
              />
            );
          })}
        </SettingsSection>

        <p className="text-xs text-neutral-500 dark:text-neutral-400 leading-relaxed px-1">
          {t('settings.notifications.categoryFooter')}
        </p>
      </div>
    </div>
  );
};

export default NotificationsPanel;
