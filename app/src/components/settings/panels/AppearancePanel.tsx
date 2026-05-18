import type { ReactElement } from 'react';

import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { setThemeMode, type ThemeMode } from '../../../store/themeSlice';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

interface ModeOption {
  id: ThemeMode;
  label: string;
  description: string;
  icon: ReactElement;
}

const SunIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden>
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M12 3v2m0 14v2m9-9h-2M5 12H3m15.364-6.364l-1.414 1.414M7.05 16.95l-1.414 1.414m12.728 0l-1.414-1.414M7.05 7.05L5.636 5.636M16 12a4 4 0 11-8 0 4 4 0 018 0z"
    />
  </svg>
);

const MoonIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden>
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M21 12.79A9 9 0 1111.21 3 7 7 0 0021 12.79z"
    />
  </svg>
);

const SystemIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden>
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M9 17v2m6-2v2m-9-2h12a2 2 0 002-2V7a2 2 0 00-2-2H6a2 2 0 00-2 2v8a2 2 0 002 2z"
    />
  </svg>
);

const OPTIONS: ModeOption[] = [
  { id: 'light', label: 'Light', description: 'Bright surfaces, dark text.', icon: SunIcon },
  {
    id: 'dark',
    label: 'Dark',
    description: 'Dim surfaces, easier on the eyes after dusk.',
    icon: MoonIcon,
  },
  {
    id: 'system',
    label: 'Match system',
    description: 'Follow your OS appearance setting.',
    icon: SystemIcon,
  },
];

const AppearancePanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const dispatch = useAppDispatch();
  const mode = useAppSelector(state => state.theme.mode);

  return (
    <div>
      <SettingsHeader
        title="Appearance"
        showBackButton
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        <div>
          <h3 className="text-xs font-semibold uppercase tracking-wider text-neutral-400 dark:text-neutral-500 mb-2 px-1">
            Theme
          </h3>
          <div
            className="bg-white dark:bg-neutral-900 rounded-xl border border-neutral-200 dark:border-neutral-800 overflow-hidden"
            role="radiogroup"
            aria-label="Theme">
            {OPTIONS.map((opt, idx) => {
              const selected = opt.id === mode;
              return (
                <button
                  key={opt.id}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  onClick={() => dispatch(setThemeMode(opt.id))}
                  className={`w-full flex items-center gap-3 px-4 py-3 text-left transition-colors focus:outline-none focus-visible:bg-primary-50 dark:bg-primary-500/10 dark:focus-visible:bg-primary-900/30 ${
                    idx !== 0 ? 'border-t border-neutral-100 dark:border-neutral-800' : ''
                  } ${
                    selected
                      ? 'bg-primary-50 dark:bg-primary-500/10'
                      : 'hover:bg-neutral-50 dark:hover:bg-neutral-800/60'
                  }`}>
                  <span
                    className={`flex items-center justify-center w-9 h-9 rounded-lg ${
                      selected
                        ? 'bg-primary-500 text-white'
                        : 'bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-300'
                    }`}>
                    {opt.icon}
                  </span>
                  <span className="flex-1 min-w-0">
                    <span className="block text-sm font-medium text-neutral-900 dark:text-neutral-100">
                      {opt.label}
                    </span>
                    <span className="block text-xs text-neutral-500 dark:text-neutral-400">
                      {opt.description}
                    </span>
                  </span>
                  {selected && (
                    <svg
                      className="w-5 h-5 text-primary-500"
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24"
                      aria-hidden>
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M5 13l4 4L19 7"
                      />
                    </svg>
                  )}
                </button>
              );
            })}
          </div>
          <p className="text-xs text-neutral-500 dark:text-neutral-400 leading-relaxed px-1 mt-2">
            Dark mode switches the entire app — chat, settings, panels — to a dim palette. "Match
            system" follows your OS appearance and updates live.
          </p>
        </div>
      </div>
    </div>
  );
};

export default AppearancePanel;
