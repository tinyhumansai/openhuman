import { type VariantProps } from 'class-variance-authority';
import { ToggleGroup as ToggleGroupPrimitive } from 'radix-ui';
import {
  type ComponentPropsWithoutRef,
  createContext,
  type ElementRef,
  forwardRef,
  useContext,
  useMemo,
} from 'react';

import { cn } from '../../lib/cn';
import { toggleVariants } from './Toggle';

/**
 * A set of toggles with one shared roving tab stop — `type="single"` behaves
 * like a segmented control, `type="multiple"` like a formatting toolbar. Items
 * reuse `Toggle`'s cva table verbatim, so a group sits beside a `Button` at the
 * same height and radius.
 *
 * The root carries the variant/size axes so a caller styles the group once
 * instead of repeating them per item; an item may still override either. The
 * shared value goes through context rather than `cloneElement` so arbitrary
 * children (a separator, a wrapper, a mapped list) keep working.
 */
type ToggleGroupContextValue = VariantProps<typeof toggleVariants>;

const ToggleGroupContext = createContext<ToggleGroupContextValue>({
  variant: 'tertiary',
  tone: 'default',
  size: 'md',
  iconOnly: false,
});

/**
 * An intersection, not an `interface extends` — `ToggleGroupPrimitive.Root`'s
 * props are a discriminated union on `type` ("single" | "multiple"), and an
 * interface cannot extend a union.
 */
export type ToggleGroupProps = ComponentPropsWithoutRef<typeof ToggleGroupPrimitive.Root> &
  VariantProps<typeof toggleVariants>;

export const ToggleGroupRoot = forwardRef<
  ElementRef<typeof ToggleGroupPrimitive.Root>,
  ToggleGroupProps
>(
  (
    {
      variant = 'tertiary',
      tone = 'default',
      size = 'md',
      iconOnly = false,
      className,
      children,
      ...rest
    },
    ref
  ) => {
    const context = useMemo(
      () => ({ variant, tone, size, iconOnly }),
      [variant, tone, size, iconOnly]
    );

    return (
      <ToggleGroupPrimitive.Root
        ref={ref}
        data-slot="toggle-group"
        data-variant={variant}
        data-tone={tone}
        data-size={size}
        className={cn('inline-flex items-center gap-1', className)}
        {...rest}>
        <ToggleGroupContext.Provider value={context}>{children}</ToggleGroupContext.Provider>
      </ToggleGroupPrimitive.Root>
    );
  }
);
ToggleGroupRoot.displayName = 'ToggleGroupRoot';

export interface ToggleGroupItemProps
  extends
    ComponentPropsWithoutRef<typeof ToggleGroupPrimitive.Item>,
    VariantProps<typeof toggleVariants> {}

export const ToggleGroupItem = forwardRef<
  ElementRef<typeof ToggleGroupPrimitive.Item>,
  ToggleGroupItemProps
>(({ variant, tone, size, iconOnly, className, ...rest }, ref) => {
  const group = useContext(ToggleGroupContext);
  // An item-level prop wins; otherwise the group's axis applies.
  const resolved = {
    variant: variant ?? group.variant,
    tone: tone ?? group.tone,
    size: size ?? group.size,
    iconOnly: iconOnly ?? group.iconOnly,
  };

  return (
    <ToggleGroupPrimitive.Item
      ref={ref}
      data-slot="toggle-group-item"
      data-variant={resolved.variant}
      data-tone={resolved.tone}
      data-size={resolved.size}
      className={cn(toggleVariants(resolved), className)}
      {...rest}
    />
  );
});
ToggleGroupItem.displayName = 'ToggleGroupItem';

export default ToggleGroupRoot;
