import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const ProfilePanel = () => {
  const { navigateBack } = useSettingsNavigation();

  return (
    <div>
      <SettingsHeader title="Profile" showBackButton={true} onBack={navigateBack} />

      <div className="py-10 flex items-center justify-center">
        <div className="text-center">
          <div className="w-12 h-12 mx-auto mb-3 bg-stone-100 rounded-full flex items-center justify-center">
            <svg
              className="w-6 h-6 text-stone-400"
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
          <p className="text-xs text-stone-500">Coming soon</p>
        </div>
      </div>
    </div>
  );
};

export default ProfilePanel;
