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
  return (
    <div className={`bg-stone-50 border-b border-stone-200 p-3 relative ${className}`}>
      <div className="flex items-center">
        {/* Back button */}
        {showBackButton && onBack && (
          <button
            onClick={onBack}
            className="w-8 h-8 flex items-center justify-center rounded-full hover:bg-stone-200 transition-colors mr-3"
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
        <h2 className="text-lg font-semibold text-stone-900">{title}</h2>
      </div>
    </div>
  );
};

export default SettingsHeader;
