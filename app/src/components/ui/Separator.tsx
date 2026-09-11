import { Separator as SeparatorPrimitive } from 'radix-ui';
import { type ComponentPropsWithoutRef } from 'react';

import { cn } from '../../lib/cn';

export type SeparatorProps = ComponentPropsWithoutRef<typeof SeparatorPrimitive.Root>;

/**
 * Radix supplies the correct semantics: a decorative separator is removed from
 * the a11y tree, a semantic one is exposed as `role="separator"`. Hand-rolled
 * `<div className="border-t" />` dividers get neither.
 */
const Separator = ({
  className,
  orientation = 'horizontal',
  decorative = true,
  ...rest
}: SeparatorProps) => (
  <SeparatorPrimitive.Root
    data-slot="separator"
    orientation={orientation}
    decorative={decorative}
    className={cn(
      'shrink-0 bg-line',
      orientation === 'horizontal' ? 'h-px w-full' : 'h-full w-px',
      className
    )}
    {...rest}
  />
);

export default Separator;
