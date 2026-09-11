import { useT } from '../../lib/i18n/I18nContext';
import { Button, TextField } from '../ui';

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
      <div className="pointer-events-none absolute inset-y-0 left-3 z-10 flex items-center">
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
      <TextField
        type="text"
        value={value}
        onChange={e => onChange(e.target.value)}
        placeholder={effectivePlaceholder}
        className="rounded-xl pl-9 pr-9"
      />
      {value && (
        <Button
          iconOnly
          variant="tertiary"
          size="sm"
          onClick={() => onChange('')}
          aria-label={t('common.clear')}
          className="absolute inset-y-0 right-1 h-auto w-8 text-content-faint hover:text-content-secondary dark:text-content-secondary">
          <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M6 18L18 6M6 6l12 12"
            />
          </svg>
        </Button>
      )}
    </div>
  );
}
