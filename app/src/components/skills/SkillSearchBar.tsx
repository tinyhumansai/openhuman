import { useT } from '../../lib/i18n/I18nContext';

interface SkillSearchBarProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
}

export default function SkillSearchBar({ value, onChange, placeholder }: SkillSearchBarProps) {
  const { t } = useT();
  const effectivePlaceholder = placeholder ?? t('skills.search.placeholder');
  return (
    <div className="relative">
      <div className="pointer-events-none absolute inset-y-0 left-3 flex items-center">
        <svg
          className="h-4 w-4 text-content-faint"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M21 21l-4.35-4.35M17 11A6 6 0 1 0 5 11a6 6 0 0 0 12 0z"
          />
        </svg>
      </div>
      <input
        type="text"
        value={value}
        onChange={e => onChange(e.target.value)}
        placeholder={effectivePlaceholder}
        className="w-full rounded-xl border border-line bg-surface py-2 pl-9 pr-9 text-sm text-content placeholder-content-faint focus:border-primary-300 focus:outline-none focus:ring-1 focus:ring-primary-200"
      />
      {value && (
        <button
          type="button"
          onClick={() => onChange('')}
          className="absolute inset-y-0 right-3 flex items-center text-content-faint hover:text-content-secondary dark:text-content-secondary">
          <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M6 18L18 6M6 6l12 12"
            />
          </svg>
        </button>
      )}
    </div>
  );
}
