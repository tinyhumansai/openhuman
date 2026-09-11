import { createContext, forwardRef, type HTMLAttributes, useContext, useMemo } from 'react';

import { cn } from '../../lib/cn';
import Button, { type ButtonProps } from './Button';
import { type InputSize } from './Input';
import TextField, { type TextFieldProps } from './TextField';

/**
 * An input with things attached to it: a unit label, a prefix like `https://`,
 * a copy button, a search icon. The group joins the borders of whatever sits
 * in it so the row reads as a single control.
 *
 * No Radix primitive — this is composition. It deliberately does **not**
 * re-implement an input: `InputGroupInput` is `TextField`, and
 * `InputGroupButton` is `Button`, so every focus/invalid/disabled behaviour and
 * every size on those two stays the one implementation.
 *
 * Position is resolved by child selectors on the root, not by each part
 * knowing where it sits, so a conditional addon or a mapped list still rounds
 * correctly. Parts must be *direct* children for that to hold.
 *
 * `size` on the root is `Input`'s size scale (`sm` | `md` | `lg`) and is passed
 * to the input and the addons through context; `InputGroupButton` maps it onto
 * `Button`'s scale so the button matches the input's height exactly.
 */
const SIZE_TO_BUTTON_SIZE: Record<InputSize, NonNullable<ButtonProps['size']>> = {
  sm: 'sm',
  md: 'md',
  lg: 'lg',
};

const ADDON_SIZES: Record<InputSize, string> = {
  sm: 'h-8 px-2.5 text-sm rounded-md',
  md: 'h-9 px-3 text-sm rounded-lg',
  lg: 'h-11 px-4 text-base rounded-lg',
};

interface InputGroupContextValue {
  size: InputSize;
}

const InputGroupContext = createContext<InputGroupContextValue>({ size: 'md' });

export interface InputGroupProps extends HTMLAttributes<HTMLDivElement> {
  size?: InputSize;
}

export const InputGroupRoot = forwardRef<HTMLDivElement, InputGroupProps>((props, ref) => {
  const { size = 'md', className, children, ...rest } = props;
  const context = useMemo(() => ({ size }), [size]);

  return (
    <InputGroupContext.Provider value={context}>
      <div
        ref={ref}
        data-slot="input-group"
        data-size={size}
        className={cn(
          'flex w-full items-stretch *:relative focus-within:*:z-10',
          '[&>*:not(:first-child)]:rounded-l-none [&>*:not(:last-child)]:rounded-r-none',
          '[&>*:not(:first-child)]:-ml-px',
          className
        )}
        {...rest}>
        {children}
      </div>
    </InputGroupContext.Provider>
  );
});
InputGroupRoot.displayName = 'InputGroupRoot';

export type InputGroupInputProps = TextFieldProps;

/** The `TextField` in the group — flexes to fill whatever the addons leave. */
export const InputGroupInput = forwardRef<HTMLInputElement, InputGroupInputProps>(
  ({ inputSize, className, ...rest }, ref) => {
    const { size } = useContext(InputGroupContext);
    return (
      <TextField
        ref={ref}
        inputSize={inputSize ?? size}
        data-slot="input-group-input"
        className={cn('min-w-0 flex-1 focus:z-10', className)}
        {...rest}
      />
    );
  }
);
InputGroupInput.displayName = 'InputGroupInput';

export interface InputGroupAddonProps extends HTMLAttributes<HTMLSpanElement> {
  size?: InputSize;
}

/**
 * Static, non-interactive content bolted to the input — a unit, a prefix, an
 * icon. `aria-hidden` is NOT applied for you: a `kg` suffix is meaningful to a
 * screen reader, a decorative magnifier is not, and only the caller knows
 * which this is.
 */
export const InputGroupAddon = forwardRef<HTMLSpanElement, InputGroupAddonProps>(
  ({ size, className, children, ...rest }, ref) => {
    const group = useContext(InputGroupContext);
    const resolved = size ?? group.size;
    return (
      <span
        ref={ref}
        data-slot="input-group-addon"
        data-size={resolved}
        className={cn(
          'inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap border border-line-strong',
          'bg-surface-muted text-content-muted',
          ADDON_SIZES[resolved],
          className
        )}
        {...rest}>
        {children}
      </span>
    );
  }
);
InputGroupAddon.displayName = 'InputGroupAddon';

export type InputGroupButtonProps = ButtonProps;

/** A `Button` sized to the group's input height. */
export const InputGroupButton = forwardRef<HTMLButtonElement, InputGroupButtonProps>(
  ({ variant, size, className, ...rest }, ref) => {
    const group = useContext(InputGroupContext);
    return (
      <Button
        ref={ref}
        variant={variant ?? 'secondary'}
        size={size ?? SIZE_TO_BUTTON_SIZE[group.size]}
        data-slot="input-group-button"
        className={cn('shrink-0 focus-visible:z-10', className)}
        {...rest}
      />
    );
  }
);
InputGroupButton.displayName = 'InputGroupButton';

export default InputGroupRoot;
