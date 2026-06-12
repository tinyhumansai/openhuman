// ---------------------------------------------------------------------------
// SettingsSearchBar
//
// Global search for the Settings tree (Phase 1 — pages / sections / dev tools).
// Renders a search input plus, while a query is active, a ranked result list.
// Selecting a result navigates to that settings route.
//
// Implemented as an ARIA combobox: the input owns `aria-activedescendant`, the
// result list is a `listbox`, and Arrow/Enter/Escape are handled on the input
// so the user never has to move focus off the field.
//
// The component is controlled (`value` / `onValueChange`) so the parent
// (SettingsHome) can hide the normal menu while a search is in progress.
// ---------------------------------------------------------------------------
import { useCallback, useId, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import { useSettingsSearch } from './useSettingsSearch';

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
  const { navigateToSettings } = useSettingsNavigation();
  const results = useSettingsSearch(value);
  const isSearching = value.trim().length > 0;

  const [activeIndex, setActiveIndex] = useState(0);
  const [lastValue, setLastValue] = useState(value);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const listboxId = useId();
  const optionId = (index: number) => `${listboxId}-opt-${index}`;

  // Reset the highlighted row whenever the query changes so the active index
  // never points past the end of the (possibly shorter) result set. Adjusting
  // state during render is React's recommended alternative to a reset effect.
  if (value !== lastValue) {
    setLastValue(value);
    setActiveIndex(0);
  }

  const goToResult = useCallback(
    (index: number) => {
      const result = results[index];
      if (!result) return;
      // [settings-search] navigate to the chosen destination and clear the query
      // so returning to the menu shows the full list again.
      onValueChange('');
      navigateToSettings(result.entry.route);
    },
    [results, navigateToSettings, onValueChange]
  );

  const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (!isSearching) {
      if (event.key === 'Escape' && value) {
        event.preventDefault();
        onValueChange('');
      }
      return;
    }

    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault();
        setActiveIndex(prev => (results.length ? (prev + 1) % results.length : 0));
        break;
      case 'ArrowUp':
        event.preventDefault();
        setActiveIndex(prev => (results.length ? (prev - 1 + results.length) % results.length : 0));
        break;
      case 'Enter':
        if (results.length) {
          event.preventDefault();
          goToResult(activeIndex);
        }
        break;
      case 'Escape':
        event.preventDefault();
        onValueChange('');
        break;
      default:
        break;
    }
  };

  return (
    <div className="px-4 pt-4" data-testid="settings-search">
      <div className="relative">
        <span className="pointer-events-none absolute inset-y-0 left-3 flex items-center text-stone-400 dark:text-neutral-500">
          <SearchIcon />
        </span>
        <input
          ref={inputRef}
          type="text"
          role="combobox"
          aria-expanded={isSearching}
          aria-controls={listboxId}
          aria-activedescendant={isSearching && results.length ? optionId(activeIndex) : undefined}
          aria-label={t('settings.settingsSearch.ariaLabel')}
          autoComplete="off"
          spellCheck={false}
          value={value}
          onChange={event => onValueChange(event.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={t('settings.settingsSearch.placeholder')}
          data-testid="settings-search-input"
          className="w-full rounded-2xl border border-stone-200 bg-white py-2.5 pl-10 pr-10 text-sm text-stone-900 placeholder:text-stone-400 focus:border-primary-400 focus:outline-none focus:ring-2 focus:ring-primary-100 dark:border-neutral-800 dark:bg-neutral-900 dark:text-neutral-100 dark:placeholder:text-neutral-500 dark:focus:ring-primary-500/20"
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

      {isSearching && (
        <div className="pt-3 pb-5">
          {results.length === 0 ? (
            <div
              role="status"
              data-testid="settings-search-empty"
              className="rounded-3xl border border-stone-200 bg-stone-50 px-4 py-6 text-center text-sm text-stone-500 dark:border-neutral-800 dark:bg-neutral-900/40 dark:text-neutral-400">
              {t('settings.settingsSearch.noResults').replace('{query}', value.trim())}
            </div>
          ) : (
            <ul
              id={listboxId}
              role="listbox"
              aria-label={t('settings.settingsSearch.resultsLabel')}
              data-testid="settings-search-results"
              className="overflow-hidden rounded-3xl border border-stone-200 dark:border-neutral-800">
              {results.map((result, index) => (
                <li
                  key={result.entry.id}
                  id={optionId(index)}
                  role="option"
                  aria-selected={index === activeIndex}
                  data-testid={`settings-search-result-${result.entry.id}`}
                  onMouseEnter={() => setActiveIndex(index)}
                  onClick={() => goToResult(index)}
                  className={`flex cursor-pointer items-center justify-between gap-3 border-b border-stone-200 px-4 py-3 last:border-b-0 dark:border-neutral-800 ${
                    index === activeIndex
                      ? 'bg-primary-50 dark:bg-primary-500/10'
                      : 'bg-stone-50 dark:bg-neutral-900/40'
                  }`}>
                  <div className="min-w-0">
                    <div className="truncate text-sm font-medium text-stone-900 dark:text-neutral-100">
                      {result.title}
                    </div>
                    {result.description && (
                      <div className="truncate text-xs text-stone-500 dark:text-neutral-400">
                        {result.description}
                      </div>
                    )}
                  </div>
                  <span className="shrink-0 rounded-full bg-stone-100 px-2 py-0.5 text-[11px] font-medium text-stone-500 dark:bg-neutral-800 dark:text-neutral-400">
                    {result.section}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
};

export default SettingsSearchBar;
