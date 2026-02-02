interface SettingsBackButtonProps {
  onClick: () => void;
  title?: string;
  className?: string;
}

const SettingsBackButton = ({
  onClick,
  title = 'Settings',
  className = '',
}: SettingsBackButtonProps) => {
  return (
    <div className={`bg-black/30 border-b border-stone-700 p-6 ${className}`}>
      <button
        onClick={onClick}
        className="flex items-center space-x-3 text-white hover:text-stone-300 transition-colors duration-150"
        aria-label="Go back">
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
        </svg>
        <span className="text-lg font-semibold">{title}</span>
      </button>
    </div>
  );
};

export default SettingsBackButton;
