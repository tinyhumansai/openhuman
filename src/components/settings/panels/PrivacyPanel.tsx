import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import SettingsHeader from '../components/SettingsHeader';

const PrivacyPanel = () => {
  const { navigateBack } = useSettingsNavigation();

  return (
    <div className="overflow-hidden h-full flex flex-col">
      <SettingsHeader
        title="Privacy & Security"
        showBackButton={true}
        onBack={navigateBack}
      />

      <div className="flex-1 overflow-y-auto">
        <div className="p-4 h-full flex items-center justify-center">
          <div className="text-center">
            <div className="w-16 h-16 mx-auto mb-4 bg-stone-700/50 rounded-full flex items-center justify-center">
              <svg className="w-8 h-8 text-stone-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
              </svg>
            </div>
            <h3 className="text-lg font-medium text-white mb-2">Privacy & Security</h3>
            <p className="text-stone-400 text-sm max-w-sm mx-auto">
              Manage your privacy settings, data retention policies, and security preferences.
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

export default PrivacyPanel;