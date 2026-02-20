import { clearToken } from '../../store/authSlice';
import { useAppDispatch } from '../../store/hooks';
import SettingsHeader from './components/SettingsHeader';
import SettingsMenuItem from './components/SettingsMenuItem';
import { useSettingsNavigation } from './hooks/useSettingsNavigation';

const SettingsHome = () => {
  const dispatch = useAppDispatch();
  const { navigateToSettings, closeSettings } = useSettingsNavigation();

  const handleLogout = async () => {
    await dispatch(clearToken());
    closeSettings();
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
    {
      id: 'skills',
      title: 'Skills',
      description: 'Configure Slack, Discord, and other skills',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M9.75 3a.75.75 0 00-1.5 0v2.25H6a2.25 2.25 0 000 4.5h2.25V12H6a2.25 2.25 0 000 4.5h2.25V18a.75.75 0 001.5 0v-1.5H12V18a.75.75 0 001.5 0v-1.5H18a2.25 2.25 0 000-4.5h-4.5V9.75H18a2.25 2.25 0 000-4.5h-4.5V3a.75.75 0 00-1.5 0v2.25H9.75V3z"
          />
        </svg>
      ),
      onClick: () => navigateToSettings('skills'),
      dangerous: false,
    },
    {
      id: 'agent-chat',
      title: 'Agent Chat',
      description: 'Send messages directly to your agent',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M8 10h8m-8 4h5m-6 6l-4 4V6a2 2 0 012-2h12a2 2 0 012 2v9a2 2 0 01-2 2H7z"
          />
        </svg>
      ),
      onClick: () => navigateToSettings('agent-chat'),
      dangerous: false,
    },
    {
      id: 'privacy',
      title: 'Privacy & Security',
      description: 'Control your privacy and security settings',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z"
          />
        </svg>
      ),
      onClick: () => navigateToSettings('privacy'),
      dangerous: false,
    },
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
      id: 'tauri-commands',
      title: 'Tauri Command Console',
      description: 'Run Alphahuman Tauri commands for quick testing',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M9 12h6m2 8H7a2 2 0 01-2-2V6a2 2 0 012-2h6l6 6v8a2 2 0 01-2 2z"
          />
        </svg>
      ),
      onClick: () => navigateToSettings('tauri-commands'),
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

      <div className="flex-1 overflow-y-auto max-w-md mx-auto">
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
    </div>
  );
};

export default SettingsHome;
