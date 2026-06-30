import { type ReactNode } from 'react';

export type SettingsBadgeVariant = 'neutral' | 'primary' | 'success' | 'warning' | 'danger';

export interface SettingsBadgeProps {
  variant: SettingsBadgeVariant;
  children: ReactNode;
  className?: string;
}

const VARIANTS: Record<SettingsBadgeVariant, string> = {
  neutral: 'bg-surface-subtle text-content-secondary border-line dark:border-line-strong',
  primary:
    'bg-primary-50 dark:bg-primary-500/10 text-primary-700 dark:text-primary-300 border-primary-200 dark:border-primary-500/30',
  success:
    'bg-sage-50 dark:bg-sage-500/10 text-sage-700 dark:text-sage-300 border-sage-200 dark:border-sage-500/30',
  warning:
    'bg-amber-50 dark:bg-amber-500/10 text-amber-700 dark:text-amber-300 border-amber-200 dark:border-amber-500/30',
  danger:
    'bg-coral-50 dark:bg-coral-500/10 text-coral-600 dark:text-coral-300 border-coral-200 dark:border-coral-500/30',
};

const SettingsBadge = ({ variant, children, className }: SettingsBadgeProps) => {
  const classes = [
    'inline-flex items-center rounded-md px-1.5 py-0.5 text-[11px] font-medium leading-none border',
    VARIANTS[variant],
    className ?? '',
  ]
    .filter(Boolean)
    .join(' ');

  return <span className={classes}>{children}</span>;
};

export default SettingsBadge;
