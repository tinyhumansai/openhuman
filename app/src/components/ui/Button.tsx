import { type ButtonHTMLAttributes, forwardRef, type ReactNode } from 'react';

export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';
export type ButtonSize = 'xs' | 'sm' | 'md' | 'lg' | 'xl';

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  leadingIcon?: ReactNode;
  trailingIcon?: ReactNode;
}

const BASE =
  'inline-flex items-center justify-center gap-2 font-medium transition-colors duration-150 ' +
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary-500/25 ' +
  'focus-visible:ring-offset-2 disabled:opacity-40 disabled:pointer-events-none';

const VARIANTS: Record<ButtonVariant, string> = {
  primary:
    'bg-primary-500 text-white hover:bg-primary-600 active:bg-primary-700 ' +
    'dark:bg-primary-500 dark:hover:bg-primary-400 dark:active:bg-primary-600',
  secondary:
    'bg-neutral-0 text-neutral-900 border border-neutral-300 hover:bg-neutral-50 ' +
    'dark:bg-neutral-900 dark:text-neutral-100 dark:border-neutral-700 dark:hover:bg-neutral-800',
  ghost:
    'bg-transparent text-neutral-700 hover:bg-neutral-100 ' +
    'dark:text-neutral-200 dark:hover:bg-neutral-800',
  danger:
    'bg-transparent text-coral-600 border border-coral-300/50 hover:bg-coral-50 ' +
    'dark:text-coral-400 dark:border-coral-500/40 dark:hover:bg-coral-500/10',
};

const SIZES: Record<ButtonSize, string> = {
  xs: 'h-6 px-2 text-xs rounded-sm',
  sm: 'h-[30px] px-3 text-sm rounded-md',
  md: 'h-9 px-4 text-sm rounded-lg',
  lg: 'h-11 px-5 text-base rounded-lg',
  xl: 'h-14 px-7 text-base rounded-xl font-medium',
};

const Button = forwardRef<HTMLButtonElement, ButtonProps>((props, ref) => {
  const {
    variant = 'primary',
    size = 'md',
    leadingIcon,
    trailingIcon,
    className,
    type,
    children,
    ...rest
  } = props;

  const classes = [BASE, VARIANTS[variant], SIZES[size], className ?? ''].filter(Boolean).join(' ');

  return (
    <button ref={ref} type={type ?? 'button'} className={classes} {...rest}>
      {leadingIcon}
      {children}
      {trailingIcon}
    </button>
  );
});
Button.displayName = 'Button';

export default Button;
