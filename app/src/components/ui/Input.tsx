import { forwardRef, type InputHTMLAttributes } from 'react';

import { cn } from '../../lib/cn';

export type InputSize = 'sm' | 'md' | 'lg';

export interface InputProps extends Omit<InputHTMLAttributes<HTMLInputElement>, 'size'> {
  inputSize?: InputSize;
  invalid?: boolean;
  monospace?: boolean;
}

const SIZES: Record<InputSize, string> = {
  sm: 'h-8 px-2.5 text-sm rounded-md',
  md: 'h-9 px-3 text-sm rounded-lg',
  lg: 'h-11 px-4 text-base rounded-lg',
};

const Input = forwardRef<HTMLInputElement, InputProps>((props, ref) => {
  const {
    inputSize = 'md',
    invalid,
    monospace,
    className,
    'aria-invalid': ariaInvalid,
    ...rest
  } = props;
  const ring = invalid
    ? 'border-coral-400 focus:border-coral-500 focus:ring-coral-500/20 dark:border-coral-500/60'
    : 'border-line-strong focus:border-primary-500 focus:ring-primary-500/20 dark:focus:border-primary-400';
  // `cn` (clsx + tailwind-merge), NOT `[...].join(' ')`.
  //
  // Joining left every default in the attribute next to whatever the caller
  // passed, so which one applied was decided by Tailwind's stylesheet ordering
  // rather than by the caller. That is not a cosmetic difference: it silently
  // ignores the override about half the time, and which half depends on where
  // the two utilities happen to sit in the generated CSS. A caller passing
  // `px-2` against `inputSize="sm"`'s `px-2.5` lost, because `px-2` is emitted
  // first; a caller passing `text-2xl` against `text-sm` won, because it is
  // emitted later. Same call site, same shape of override, opposite outcomes.
  //
  // Found via the flow-canvas title, which had to stop using this component to
  // get its own font size to stick. Every other primitive in `ui/` already
  // composes through `cn` — this one was the outlier.
  const classes = cn(
    'w-full border bg-surface text-content placeholder-content-faint',
    'transition-colors duration-150 focus:outline-hidden focus:ring-2',
    'disabled:opacity-50 disabled:bg-surface-muted',
    SIZES[inputSize],
    ring,
    monospace && 'font-mono',
    className
  );
  return (
    <input ref={ref} className={classes} {...rest} aria-invalid={invalid ? true : ariaInvalid} />
  );
});
Input.displayName = 'Input';

export default Input;
