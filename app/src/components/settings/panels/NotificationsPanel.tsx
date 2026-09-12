import { useT } from '../../../lib/i18n/I18nContext';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { type NotificationCategory, setPreference } from '../../../store/notificationSlice';
import { SettingsRow, SettingsSection, SettingsSwitch } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

interface NotificationsPanelProps {
  /** When embedded inside the tabbed Notifications page, the parent owns the
      `<SettingsHeader>` chrome and we render only the body. */
  embedded?: boolean;
}

const CATEGORIES: { id: NotificationCategory; titleKey: string; descKey: string }[] = [
  {
    id: 'messages',
    titleKey: 'settings.notifications.category.messages.title',
    descKey: 'settings.notifications.category.messages.desc',
  },
  {
    id: 'agents',
    titleKey: 'settings.notifications.category.agents.title',
    descKey: 'settings.notifications.category.agents.desc',
  },
  {
    id: 'skills',
    titleKey: 'settings.notifications.category.skills.title',
    descKey: 'settings.notifications.category.skills.desc',
  },
  {
    id: 'system',
    titleKey: 'settings.notifications.category.system.title',
    descKey: 'settings.notifications.category.system.desc',
  },
  {
    id: 'meetings',
    titleKey: 'settings.notifications.category.meetings.title',
    descKey: 'settings.notifications.category.meetings.desc',
  },
  {
    id: 'reminders',
    titleKey: 'settings.notifications.category.reminders.title',
    descKey: 'settings.notifications.category.reminders.desc',
  },
  {
    id: 'important',
    titleKey: 'settings.notifications.category.important.title',
    descKey: 'settings.notifications.category.important.desc',
  },
];

const NotificationsPanel = ({ embedded = false }: NotificationsPanelProps = {}) => {
  const { t } = useT();
  const preferences = useAppSelector(s => s.notifications.preferences);
  const dispatch = useAppDispatch();
  const handleToggle = (category: NotificationCategory) => {
    dispatch(setPreference({ category, enabled: !preferences[category] }));
  };

  const body = (
    <>
      {/* Categories */}
      <SettingsSection title={t('settings.notifications.categories')}>
        {CATEGORIES.map(cat => {
          const enabled = preferences[cat.id];
          const switchId = `switch-notif-${cat.id}`;
          const title = t(cat.titleKey);
          return (
            <SettingsRow
              key={cat.id}
              htmlFor={switchId}
              label={title}
              description={t(cat.descKey)}
              control={
                <SettingsSwitch
                  id={switchId}
                  checked={enabled}
                  onCheckedChange={() => handleToggle(cat.id)}
                  aria-label={t('settings.notifications.categoryToggleAria').replace(
                    '{name}',
                    title
                  )}
                />
              }
            />
          );
        })}
      </SettingsSection>

      <p className="text-xs text-content-muted leading-relaxed px-1">
        {t('settings.notifications.categoryFooter')}
      </p>
    </>
  );

  // `embedded` renders the body with no page chrome, for a host that draws its
  // own header. The tabbed Notifications page that used it is gone — the
  // routing tab was removed with `NotificationRoutingPanel`, and a two-tab page
  // with one tab left is a control that cannot do anything — so this route now
  // renders the preferences directly. The prop is kept for the next host.
  if (embedded) return <div className="space-y-4">{body}</div>;

  return <SettingsPanel>{body}</SettingsPanel>;
};

export default NotificationsPanel;
