export interface SettingsSwitchProps {
  id: string;
  checked: boolean;
  onCheckedChange: (next: boolean) => void;
  disabled?: boolean;
  'aria-label'?: string;
  'data-testid'?: string;
}

const SettingsSwitch = ({
  id,
  checked,
  onCheckedChange,
  disabled = false,
  'aria-label': ariaLabel,
  'data-testid': testId,
}: SettingsSwitchProps) => {
  const trackBase =
    'relative inline-flex h-[22px] w-[38px] flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent ' +
    'transition-colors duration-200 ease-in-out ' +
    'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary-500 focus-visible:ring-offset-2 ' +
    'focus-visible:ring-offset-white dark:focus-visible:ring-offset-neutral-900 ' +
    'motion-reduce:transition-none ' +
    'disabled:cursor-not-allowed disabled:opacity-50';

  const trackColor = checked ? 'bg-primary-500' : 'bg-surface-strong';

  const thumbBase =
    'pointer-events-none inline-block h-[18px] w-[18px] transform rounded-full bg-surface shadow-sm ring-0 ' +
    'transition-transform duration-200 ease-in-out motion-reduce:transition-none';

  const thumbPosition = checked ? 'translate-x-[16px]' : 'translate-x-0';

  return (
    <button
      id={id}
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      disabled={disabled}
      data-testid={testId}
      onClick={() => {
        if (!disabled) onCheckedChange(!checked);
      }}
      className={[trackBase, trackColor].join(' ')}>
      <span className={[thumbBase, thumbPosition].join(' ')} />
    </button>
  );
};

export default SettingsSwitch;
