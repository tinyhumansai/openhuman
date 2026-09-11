import { Tabs as TabsPrimitive } from 'radix-ui';
import { type ComponentPropsWithoutRef } from 'react';

import { cn } from '../../lib/cn';

export const TabsRoot = TabsPrimitive.Root;

/** Roving-focus tab list: arrow keys move, Home/End jump, only one tab stop. */
export const TabsList = ({
  className,
  variant = 'default',
  ...rest
}: ComponentPropsWithoutRef<typeof TabsPrimitive.List> & { variant?: 'default' | 'line' }) => (
  <TabsPrimitive.List
    data-slot="tabs-list"
    data-variant={variant}
    className={cn(
      'flex items-center gap-1',
      variant === 'line' && [
        'gap-0 border-b border-line',
        '**:data-[slot=tabs-trigger]:-mb-px **:data-[slot=tabs-trigger]:rounded-none',
        '**:data-[slot=tabs-trigger]:border-b-2 **:data-[slot=tabs-trigger]:border-transparent',
        'data-[state=active]:**:data-[slot=tabs-trigger]:border-primary-500',
        'data-[state=active]:**:data-[slot=tabs-trigger]:bg-transparent',
        'data-[state=active]:**:data-[slot=tabs-trigger]:text-primary-600',
        'dark:data-[state=active]:**:data-[slot=tabs-trigger]:text-primary-300',
      ],
      className
    )}
    {...rest}
  />
);

export const TabsTrigger = ({
  className,
  ...rest
}: ComponentPropsWithoutRef<typeof TabsPrimitive.Trigger>) => (
  <TabsPrimitive.Trigger
    data-slot="tabs-trigger"
    className={cn(
      'inline-flex items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-medium transition-colors',
      'text-content-muted hover:bg-surface-hover hover:text-content-secondary',
      'focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500/25',
      'data-[state=active]:bg-surface-strong data-[state=active]:text-content',
      className
    )}
    {...rest}
  />
);

export const TabsContent = ({
  className,
  ...rest
}: ComponentPropsWithoutRef<typeof TabsPrimitive.Content>) => (
  <TabsPrimitive.Content
    data-slot="tabs-content"
    className={cn('focus:outline-hidden', className)}
    {...rest}
  />
);

export default TabsRoot;
