import { forwardRef, type HTMLAttributes, type ReactNode } from 'react';

export type CardVariant = 'surface' | 'elevated' | 'outlined' | 'subtle';
export type CardPadding = 'none' | 'sm' | 'md' | 'lg';

export interface CardProps extends HTMLAttributes<HTMLDivElement> {
  variant?: CardVariant;
  padding?: CardPadding;
  children?: ReactNode;
}

const VARIANTS: Record<CardVariant, string> = {
  surface: 'bg-white border border-neutral-200 dark:bg-neutral-900 dark:border-neutral-800',
  elevated:
    'bg-white border border-neutral-200 shadow-soft ' +
    'dark:bg-neutral-900 dark:border-neutral-800 dark:shadow-none',
  outlined: 'bg-transparent border border-neutral-200 dark:border-neutral-800',
  subtle: 'bg-neutral-50 border border-neutral-100 dark:bg-neutral-900/50 dark:border-neutral-800',
};

const PADDINGS: Record<CardPadding, string> = { none: '', sm: 'p-3', md: 'p-4', lg: 'p-6' };

const Card = forwardRef<HTMLDivElement, CardProps>((props, ref) => {
  const { variant = 'surface', padding = 'md', className, children, ...rest } = props;
  const classes = [
    'rounded-xl text-neutral-900 dark:text-neutral-100',
    VARIANTS[variant],
    PADDINGS[padding],
    className ?? '',
  ]
    .filter(Boolean)
    .join(' ');
  return (
    <div ref={ref} className={classes} {...rest}>
      {children}
    </div>
  );
});
Card.displayName = 'Card';

export default Card;
