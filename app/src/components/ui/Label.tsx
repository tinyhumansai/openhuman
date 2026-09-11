import { Label as LabelPrimitive } from 'radix-ui';
import { type ComponentPropsWithoutRef } from 'react';

import { cn } from '../../lib/cn';

export type LabelProps = ComponentPropsWithoutRef<typeof LabelPrimitive.Root>;

/** Radix `Label` also forwards clicks to the associated control on iOS Safari. */
const Label = ({ className, ...rest }: LabelProps) => (
  <LabelPrimitive.Root
    data-slot="label"
    className={cn('text-sm font-medium text-content', className)}
    {...rest}
  />
);

export default Label;
