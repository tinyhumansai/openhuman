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
    <div className={`px-5 pt-5 pb-3 ${className}`}>
      <button
        onClick={onClick}
        className="flex items-center space-x-2 text-stone-900 hover:text-stone-700 transition-colors duration-150"
        aria-label="Go back">
        <svg
          className="w-4 h-4 text-stone-500"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
        </svg>
        <span className="text-sm font-semibold">{title}</span>
      </button>
    </div>
  );
};

export default SettingsBackButton;
