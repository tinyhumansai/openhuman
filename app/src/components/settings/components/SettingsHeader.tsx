interface BreadcrumbItem {
  label: string;
  onClick?: () => void;
}

interface SettingsHeaderProps {
  className?: string;
  title?: string;
  showBackButton?: boolean;
  onBack?: () => void;
  breadcrumbs?: BreadcrumbItem[];
}

const SettingsHeader = ({
  className = '',
  title = 'Settings',
  showBackButton = false,
  onBack,
  breadcrumbs,
}: SettingsHeaderProps) => {
  return (
    <div className={`px-5 pt-5 pb-3 ${className}`}>
      <div className="flex items-center">
        {/* Back button */}
        {showBackButton && onBack && (
          <button
            onClick={onBack}
            className="w-6 h-6 flex items-center justify-center rounded-full hover:bg-stone-100 transition-colors mr-2"
            aria-label="Go back">
            <svg
              className="w-4 h-4 text-stone-500"
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

        {/* Breadcrumbs */}
        {breadcrumbs && breadcrumbs.length > 0 && (
          <nav aria-label="Breadcrumb" className="mr-1">
            <ol className="flex items-center gap-1">
              {breadcrumbs.map((crumb, i) => (
                <li key={i} className="flex items-center gap-1">
                  {crumb.onClick ? (
                    <button
                      onClick={crumb.onClick}
                      className="text-xs text-stone-400 hover:text-stone-600 transition-colors">
                      {crumb.label}
                    </button>
                  ) : (
                    <span className="text-xs text-stone-400">{crumb.label}</span>
                  )}
                  <svg
                    aria-hidden="true"
                    className="w-3 h-3 text-stone-300"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M9 5l7 7-7 7"
                    />
                  </svg>
                </li>
              ))}
            </ol>
          </nav>
        )}

        {/* Title */}
        <h2 className="text-sm font-semibold text-stone-900">{title}</h2>
      </div>
    </div>
  );
};

export default SettingsHeader;
