import { forwardRef, type InputHTMLAttributes } from 'react';

export type InputSize = 'sm' | 'md' | 'lg';

export interface InputProps extends Omit<InputHTMLAttributes<HTMLInputElement>, 'size'> {
  inputSize?: InputSize;
  invalid?: boolean;
}

const SIZES: Record<InputSize, string> = {
  sm: 'h-8 px-2.5 text-sm rounded-md',
  md: 'h-9 px-3 text-sm rounded-lg',
  lg: 'h-11 px-4 text-base rounded-lg',
};

const Input = forwardRef<HTMLInputElement, InputProps>((props, ref) => {
  const { inputSize = 'md', invalid, className, ...rest } = props;
  const ring = invalid
    ? 'border-coral-400 focus:border-coral-500 focus:ring-coral-500/20 dark:border-coral-500/60'
    : 'border-neutral-300 focus:border-primary-500 focus:ring-primary-500/20 dark:border-neutral-700 dark:focus:border-primary-400';
  const classes = [
    'w-full border bg-white text-neutral-900 placeholder-neutral-400',
    'transition-colors duration-150 focus:outline-none focus:ring-2',
    'disabled:opacity-50 disabled:bg-neutral-50',
    'dark:bg-neutral-900 dark:text-neutral-100 dark:placeholder-neutral-500 dark:disabled:bg-neutral-800',
    SIZES[inputSize],
    ring,
    className ?? '',
  ]
    .filter(Boolean)
    .join(' ');
  return <input ref={ref} className={classes} {...rest} />;
});
Input.displayName = 'Input';

export default Input;
