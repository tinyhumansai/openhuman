import { forwardRef, type HTMLAttributes, type ReactNode } from 'react';

export type CardVariant = 'surface' | 'elevated' | 'outlined' | 'subtle';
export type CardPadding = 'none' | 'sm' | 'md' | 'lg';

export interface CardProps extends HTMLAttributes<HTMLDivElement> {
  variant?: CardVariant;
  padding?: CardPadding;
  children?: ReactNode;
}

const VARIANTS: Record<CardVariant, string> = {
  surface: 'bg-surface border border-line',
  elevated: 'bg-surface border border-line shadow-soft dark:shadow-none',
  outlined: 'bg-transparent border border-line',
  subtle: 'bg-surface-muted border border-line-subtle',
};

const PADDINGS: Record<CardPadding, string> = { none: '', sm: 'p-3', md: 'p-4', lg: 'p-6' };

const Card = forwardRef<HTMLDivElement, CardProps>((props, ref) => {
  const { variant = 'surface', padding = 'md', className, children, ...rest } = props;
  const classes = ['rounded-xl text-content', VARIANTS[variant], PADDINGS[padding], className ?? '']
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
