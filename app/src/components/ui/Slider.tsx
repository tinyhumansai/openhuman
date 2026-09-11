import { cva, type VariantProps } from 'class-variance-authority';
import { Slider as SliderPrimitive } from 'radix-ui';
import { type ComponentPropsWithoutRef, forwardRef } from 'react';

import { cn } from '../../lib/cn';

/**
 * A range control on Radix `Slider`.
 *
 * What the primitive buys over an `<input type="range">` with a repainted
 * thumb: real keyboard semantics (arrows step, Home/End jump, PageUp/PageDown
 * take a larger step), `aria-valuenow`/`aria-orientation` maintained for us,
 * and multi-thumb ranges — a two-value `<input type="range">` does not exist.
 *
 * The thumb count is derived from `value`/`defaultValue`, so a range slider is
 * just a two-element array; each thumb below is rendered from that array rather
 * than hardcoded, otherwise the second handle would be unreachable.
 */
export const sliderTrackVariants = cva(
  'relative grow overflow-hidden rounded-full bg-surface-strong',
  {
    variants: {
      size: {
        sm: 'h-1 data-[orientation=vertical]:h-full data-[orientation=vertical]:w-1',
        md: 'h-1.5 data-[orientation=vertical]:h-full data-[orientation=vertical]:w-1.5',
      },
    },
    defaultVariants: { size: 'md' },
  }
);

export const sliderThumbVariants = cva(
  'block rounded-full border-2 border-primary-500 bg-surface shadow-xs transition-colors ' +
    'focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500/25 ' +
    'disabled:pointer-events-none data-disabled:opacity-50',
  { variants: { size: { sm: 'h-3 w-3', md: 'h-4 w-4' } }, defaultVariants: { size: 'md' } }
);

export type SliderSize = NonNullable<VariantProps<typeof sliderThumbVariants>['size']>;

export interface SliderProps
  extends
    Omit<ComponentPropsWithoutRef<typeof SliderPrimitive.Root>, 'children'>,
    VariantProps<typeof sliderThumbVariants> {
  /** Per-thumb accessible name. Index-aligned with `value`/`defaultValue`. */
  thumbLabels?: string[];
  'data-testid'?: string;
}

const Slider = forwardRef<HTMLSpanElement, SliderProps>(
  (
    { size = 'md', className, value, defaultValue, thumbLabels, 'data-testid': testId, ...rest },
    ref
  ) => {
    const thumbCount = (value ?? defaultValue ?? [0]).length;

    return (
      <SliderPrimitive.Root
        ref={ref}
        data-slot="slider"
        data-size={size}
        data-testid={testId}
        value={value}
        defaultValue={defaultValue}
        className={cn(
          'relative flex w-full touch-none select-none items-center',
          'data-[orientation=vertical]:h-full data-[orientation=vertical]:w-auto data-[orientation=vertical]:flex-col',
          'data-disabled:opacity-50',
          className
        )}
        {...rest}>
        <SliderPrimitive.Track data-slot="slider-track" className={sliderTrackVariants({ size })}>
          <SliderPrimitive.Range
            data-slot="slider-range"
            className="absolute h-full bg-primary-500 data-[orientation=vertical]:w-full"
          />
        </SliderPrimitive.Track>
        {Array.from({ length: thumbCount }, (_, index) => (
          <SliderPrimitive.Thumb
            key={index}
            data-slot="slider-thumb"
            aria-label={thumbLabels?.[index]}
            className={sliderThumbVariants({ size })}
          />
        ))}
      </SliderPrimitive.Root>
    );
  }
);
Slider.displayName = 'Slider';

export default Slider;
