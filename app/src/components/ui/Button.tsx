import { type ButtonHTMLAttributes, forwardRef, type ReactNode } from 'react';

/**
 * The one button in the app. Three hierarchy variants plus an orthogonal tone:
 *
 * - **variant** — visual weight / importance:
 *   - `primary`   the main call-to-action on a surface (Save, Continue, Create)
 *   - `secondary` an alternative of similar weight (Cancel, Back, Import)
 *   - `tertiary`  low-emphasis / text-style action (Skip, links, inline actions)
 * - **tone** — semantic intent layered on any variant:
 *   - `default`   the normal palette
 *   - `danger`    destructive actions (Delete, Remove, Logout) — coral
 *
 * Use `iconOnly` for icon-only affordances (close / refresh / add); it squares
 * the padding — always pass an `aria-label` in that case.
 */
export type ButtonVariant = 'primary' | 'secondary' | 'tertiary';
export type ButtonTone = 'default' | 'danger';
export type ButtonSize = 'xs' | 'sm' | 'md' | 'lg' | 'xl';

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  tone?: ButtonTone;
  size?: ButtonSize;
  /** Square the button for a single centered icon. Requires an `aria-label`. */
  iconOnly?: boolean;
  leadingIcon?: ReactNode;
  trailingIcon?: ReactNode;
}

const BASE =
  'inline-flex items-center justify-center gap-2 font-medium transition-colors duration-150 ' +
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-offset-2 ' +
  'disabled:opacity-40 disabled:pointer-events-none';

// variant × tone → surface classes. The focus ring colour follows the tone.
const VARIANTS: Record<ButtonVariant, Record<ButtonTone, string>> = {
  primary: {
    default:
      'bg-primary-500 text-content-inverted hover:bg-primary-600 active:bg-primary-700 focus-visible:ring-primary-500/25 ' +
      'dark:hover:bg-primary-400 dark:active:bg-primary-600',
    danger:
      'bg-coral-500 text-content-inverted hover:bg-coral-600 active:bg-coral-700 focus-visible:ring-coral-500/25 ' +
      'dark:hover:bg-coral-400 dark:active:bg-coral-600',
  },
  secondary: {
    default:
      'bg-surface text-content border border-line-strong hover:bg-surface-hover focus-visible:ring-primary-500/25',
    danger:
      'bg-transparent text-coral-600 border border-coral-300/50 hover:bg-coral-50 focus-visible:ring-coral-500/25 ' +
      'dark:text-coral-400 dark:border-coral-500/40 dark:hover:bg-coral-500/10',
  },
  tertiary: {
    default:
      'bg-transparent text-content-secondary hover:bg-surface-hover focus-visible:ring-primary-500/25',
    danger:
      'bg-transparent text-coral-600 hover:bg-coral-50 focus-visible:ring-coral-500/25 ' +
      'dark:text-coral-400 dark:hover:bg-coral-500/10',
  },
};

const SIZES: Record<ButtonSize, string> = {
  xs: 'h-6 px-2 text-xs rounded-sm',
  sm: 'h-[30px] px-3 text-sm rounded-md',
  md: 'h-9 px-4 text-sm rounded-lg',
  lg: 'h-11 px-5 text-base rounded-lg',
  xl: 'h-14 px-7 text-base rounded-xl font-medium',
};

// Square footprints for icon-only buttons (no horizontal padding).
const ICON_SIZES: Record<ButtonSize, string> = {
  xs: 'h-6 w-6 text-xs rounded-sm',
  sm: 'h-[30px] w-[30px] text-sm rounded-md',
  md: 'h-9 w-9 text-sm rounded-lg',
  lg: 'h-11 w-11 text-base rounded-lg',
  xl: 'h-14 w-14 text-base rounded-xl',
};

const Button = forwardRef<HTMLButtonElement, ButtonProps>((props, ref) => {
  const {
    variant = 'primary',
    tone = 'default',
    size = 'md',
    iconOnly = false,
    leadingIcon,
    trailingIcon,
    className,
    type,
    children,
    ...rest
  } = props;

  const sizeClass = (iconOnly ? ICON_SIZES : SIZES)[size];
  const classes = [BASE, VARIANTS[variant][tone], sizeClass, className ?? '']
    .filter(Boolean)
    .join(' ');

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
