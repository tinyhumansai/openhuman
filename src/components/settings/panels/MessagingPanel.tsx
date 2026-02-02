import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const MessagingPanel = () => {
  const { navigateBack } = useSettingsNavigation();

  return (
    <div className="overflow-hidden h-full flex flex-col">
      <SettingsHeader title="Messaging" showBackButton={true} onBack={navigateBack} />

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
                  d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"
                />
              </svg>
            </div>
            <h3 className="text-lg font-medium text-white mb-2">Messaging Settings</h3>
            <p className="text-stone-400 text-sm max-w-sm mx-auto">
              Configure your messaging preferences, notifications, and communication settings.
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

export default MessagingPanel;
