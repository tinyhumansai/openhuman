import type { Locale } from '../lib/i18n/types';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import { setLocale } from '../store/localeSlice';

const LOCALE_OPTIONS: Array<{ value: Locale; flag: string; label: string }> = [
  { value: 'en', flag: '🇬🇧', label: 'English' },
  { value: 'zh-CN', flag: '🇨🇳', label: '中文' },
];

interface LanguageSelectProps {
  /** Accessible label for the underlying <select>. */
  ariaLabel?: string;
  /** Optional id for label association. */
  id?: string;
  /** Override the default classNames if a host needs different padding/size. */
  className?: string;
}

const DEFAULT_CLASS =
  "appearance-none rounded-lg border border-stone-200 bg-white bg-[url('data:image/svg+xml;utf8,<svg%20xmlns=%22http://www.w3.org/2000/svg%22%20viewBox=%220%200%2020%2020%22%20fill=%22%2378716c%22><path%20d=%22M5.293%207.293a1%201%200%20011.414%200L10%2010.586l3.293-3.293a1%201%200%20111.414%201.414l-4%204a1%201%200%2001-1.414%200l-4-4a1%201%200%20010-1.414z%22/></svg>')] bg-no-repeat bg-[right_0.5rem_center] bg-[length:1rem_1rem] py-2 pl-3 pr-8 text-xs font-medium text-stone-700 hover:border-stone-300 hover:bg-stone-50 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500 cursor-pointer";

/**
 * Shared language picker used by the boot-check gate and the Settings home
 * screen. Renders the locale options as flag + label pairs and dispatches
 * `setLocale` to the redux store on change.
 */
const LanguageSelect = ({ ariaLabel = 'Language', id, className }: LanguageSelectProps) => {
  const dispatch = useAppDispatch();
  const current = useAppSelector(state => state.locale.current);

  return (
    <select
      id={id}
      value={current}
      onChange={e => dispatch(setLocale(e.target.value as Locale))}
      aria-label={ariaLabel}
      className={className ?? DEFAULT_CLASS}>
      {LOCALE_OPTIONS.map(opt => (
        <option key={opt.value} value={opt.value}>
          {opt.flag} {opt.label}
        </option>
      ))}
    </select>
  );
};

export default LanguageSelect;
