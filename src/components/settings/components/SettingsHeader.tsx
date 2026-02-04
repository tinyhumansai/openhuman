import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

interface SettingsHeaderProps {
  className?: string;
  title?: string;
  showBackButton?: boolean;
  onBack?: () => void;
}

const SettingsHeader = ({
  className = '',
  title = 'Settings',
  showBackButton = false,
  onBack,
}: SettingsHeaderProps) => {
  const { closeSettings } = useSettingsNavigation();

  return (
    <div className={`bg-black/30 border-b border-stone-700 p-6 relative ${className}`}>
      <div className="flex items-center justify-between">
        <div className="flex items-center">
          {/* Back button */}
          {showBackButton && onBack && (
            <button
              onClick={onBack}
              className="w-8 h-8 flex items-center justify-center rounded-full hover:bg-stone-800/50 transition-colors mr-3"
              aria-label="Go back">
              <svg
                className="w-5 h-5 opacity-70"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M15 19l-7-7 7-7"
                />
              </svg>
            </button>
          )}

          {/* Title */}
          <h2 className="text-lg font-semibold text-white" id="settings-modal-title">
            {title}
          </h2>
        </div>

        {/* Close button */}
        <button
          onClick={closeSettings}
          className="w-8 h-8 flex items-center justify-center rounded-full hover:bg-stone-800/50 transition-colors"
          aria-label="Close settings">
          <svg className="w-5 h-5 opacity-70" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M6 18L18 6M6 6l12 12"
            />
          </svg>
        </button>
      </div>
    </div>
  );
};

export default SettingsHeader;
