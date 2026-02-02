import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const ProfilePanel = () => {
  const { navigateBack } = useSettingsNavigation();

  return (
    <div className="overflow-hidden h-full flex flex-col">
      <SettingsHeader title="Profile" showBackButton={true} onBack={navigateBack} />

      <div className="flex-1 overflow-y-auto">
        <div className="p-4 h-full flex items-center justify-center">
          <div className="text-center">
            <div className="w-16 h-16 mx-auto mb-4 bg-stone-700/50 rounded-full flex items-center justify-center">
              <svg
                className="w-8 h-8 text-stone-400"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z"
                />
              </svg>
            </div>
            <h3 className="text-lg font-medium text-white mb-2">Profile Settings</h3>
            <p className="text-stone-400 text-sm max-w-sm mx-auto">
              Update your profile information, avatar, and personal preferences.
            </p>
            <div className="mt-6">
              <span className="px-4 py-2 text-sm font-medium rounded-full border bg-stone-700/30 text-stone-300 border-stone-600/50">
                Coming Soon
              </span>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default ProfilePanel;
