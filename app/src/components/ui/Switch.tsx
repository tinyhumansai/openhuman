import { Switch as SwitchPrimitive } from 'radix-ui';

import { cn } from '../../lib/cn';

export interface SwitchProps {
  id: string;
  checked: boolean;
  onCheckedChange: (next: boolean) => void;
  disabled?: boolean;
  className?: string;
  'aria-label'?: string;
  'data-testid'?: string;
}

/**
 * Radix `Switch` on the historical `SettingsSwitch` visual. What the hand-rolled
 * version could not offer: a hidden native input for form participation, and
 * `data-state` so the thumb position is driven by the primitive rather than by
 * re-deriving `checked` in the class string.
 *
 * The focus ring offsets against `surface`, not the previous
 * `ring-offset-white dark:ring-offset-neutral-900` — a hardcoded white offset
 * does not follow a user's custom theme, so the ring read wrong on any tinted
 * surface.
 */
const Switch = ({
  id,
  checked,
  onCheckedChange,
  disabled = false,
  className,
  'aria-label': ariaLabel,
  'data-testid': testId,
}: SwitchProps) => (
  <SwitchPrimitive.Root
    id={id}
    data-slot="switch"
    data-testid={testId}
    checked={checked}
    onCheckedChange={onCheckedChange}
    disabled={disabled}
    aria-label={ariaLabel}
    className={cn(
      'relative inline-flex h-[22px] w-[38px] shrink-0 cursor-pointer rounded-full border-2 border-transparent',
      'transition-colors duration-200 ease-in-out motion-reduce:transition-none',
      'focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500 focus-visible:ring-offset-2 focus-visible:ring-offset-surface',
      'disabled:cursor-not-allowed disabled:opacity-50',
      'data-[state=checked]:bg-primary-500 data-[state=unchecked]:bg-surface-strong',
      className
    )}>
    <SwitchPrimitive.Thumb
      className={cn(
        'pointer-events-none inline-block h-[18px] w-[18px] transform rounded-full bg-surface shadow-xs ring-0',
        'transition-transform duration-200 ease-in-out motion-reduce:transition-none',
        'data-[state=checked]:translate-x-[16px] data-[state=unchecked]:translate-x-0'
      )}
    />
  </SwitchPrimitive.Root>
);

export default Switch;
