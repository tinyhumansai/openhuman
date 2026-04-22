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
];

const NotificationsPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const preferences = useAppSelector(s => s.notifications.preferences);
  const dispatch = useAppDispatch();

  const handleToggle = (category: NotificationCategory) => {
    dispatch(setPreference({ category, enabled: !preferences[category] }));
  };

  return (
    <div>
      <SettingsHeader
        title="Notifications"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div>
        <div className="p-4 space-y-4">
          <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-400 mb-2 px-1">
            Categories
          </h3>
          <div className="bg-white rounded-xl border border-stone-200 overflow-hidden divide-y divide-stone-100">
            {CATEGORIES.map(cat => {
              const enabled = preferences[cat.id];
              return (
                <div key={cat.id} className="flex items-center justify-between p-4">
                  <div className="flex-1 mr-4">
                    <p className="text-sm font-medium text-stone-900">{cat.title}</p>
                    <p className="text-xs text-stone-500 mt-1 leading-relaxed">{cat.description}</p>
                  </div>
                  <button
                    onClick={() => handleToggle(cat.id)}
                    className={`relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
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

          <p className="text-xs text-stone-500 leading-relaxed px-1">
            Disabling a category stops new notifications of that type from appearing in the
            notification center. Existing notifications remain until cleared.
          </p>
        </div>
      </div>
    </div>
  );
};

export default NotificationsPanel;
