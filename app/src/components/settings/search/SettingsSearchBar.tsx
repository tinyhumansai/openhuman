// ---------------------------------------------------------------------------
// SettingsSearchBar
//
// A plain, full-width search field for the settings sidebar. It is purely a
// controlled text input — it does NOT render its own result list. The parent
// (SettingsSidebar) uses the query to filter the visible nav tabs in place.
// ---------------------------------------------------------------------------
import { useRef } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';

interface SettingsSearchBarProps {
  value: string;
  onValueChange: (next: string) => void;
}

const SearchIcon = () => (
  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M21 21l-4.35-4.35M11 19a8 8 0 100-16 8 8 0 000 16z"
    />
  </svg>
);

const ClearIcon = () => (
  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
  </svg>
);

const SettingsSearchBar = ({ value, onValueChange }: SettingsSearchBarProps) => {
  const { t } = useT();
  const inputRef = useRef<HTMLInputElement | null>(null);

  return (
    <div data-testid="settings-search" className="relative shrink-0">
      <span className="pointer-events-none absolute inset-y-0 left-3 flex items-center text-stone-400 dark:text-neutral-500">
        <SearchIcon />
      </span>
      <input
        ref={inputRef}
        type="text"
        aria-label={t('settings.settingsSearch.ariaLabel')}
        autoComplete="off"
        spellCheck={false}
        value={value}
        onChange={event => onValueChange(event.target.value)}
        onKeyDown={event => {
          if (event.key === 'Escape' && value) {
            event.preventDefault();
            onValueChange('');
          }
        }}
        placeholder={t('settings.settingsSearch.placeholder')}
        data-testid="settings-search-input"
        className="w-full border-0 border-b border-stone-200 bg-transparent py-2.5 pl-10 pr-10 text-sm text-stone-900 placeholder:text-stone-400 focus:border-primary-400 focus:outline-none focus:ring-0 dark:border-neutral-800 dark:text-neutral-100 dark:placeholder:text-neutral-500"
      />
      {value && (
        <button
          type="button"
          onClick={() => {
            onValueChange('');
            inputRef.current?.focus();
          }}
          aria-label={t('settings.settingsSearch.clear')}
          data-testid="settings-search-clear"
          className="absolute inset-y-0 right-2 flex items-center px-1.5 text-stone-400 hover:text-stone-600 dark:text-neutral-500 dark:hover:text-neutral-300">
          <ClearIcon />
        </button>
      )}
    </div>
  );
};

export default SettingsSearchBar;
