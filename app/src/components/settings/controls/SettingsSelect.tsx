import { forwardRef, type SelectHTMLAttributes } from 'react';

export type SettingsSelectSize = 'sm' | 'md';

export interface SettingsSelectProps extends SelectHTMLAttributes<HTMLSelectElement> {
  inputSize?: SettingsSelectSize;
  'data-testid'?: string;
}

// Inline chevron SVG as a background-image data-URI (stroke #a3a3a3 = neutral-400)
const CHEVRON_BG =
  "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 12 12'%3E%3Cpath d='M2 4l4 4 4-4' stroke='%23a3a3a3' stroke-width='1.5' stroke-linecap='round' stroke-linejoin='round' fill='none'/%3E%3C/svg%3E\")";

const SettingsSelect = forwardRef<HTMLSelectElement, SettingsSelectProps>(
  ({ inputSize = 'md', className, 'data-testid': testId, style, ...rest }, ref) => {
    const sizeClass = inputSize === 'sm' ? 'h-8 pl-2.5' : 'h-9 pl-3';

    const classes = [
      'block border border-line-strong',
      'bg-surface',
      'text-content',
      'text-sm rounded-lg',
      'focus:outline-none focus:border-primary-500 focus:ring-2 focus:ring-primary-500/20',
      'transition-colors duration-150',
      'cursor-pointer appearance-none bg-no-repeat pr-7',
      sizeClass,
      'disabled:opacity-50 disabled:cursor-not-allowed',
      className ?? '',
    ]
      .filter(Boolean)
      .join(' ');

    return (
      <select
        ref={ref}
        data-testid={testId}
        className={classes}
        style={{
          backgroundImage: CHEVRON_BG,
          backgroundPosition: 'right 0.5rem center',
          backgroundSize: '12px 12px',
          ...style,
        }}
        {...rest}
      />
    );
  }
);
SettingsSelect.displayName = 'SettingsSelect';

export default SettingsSelect;
