import { type ReactNode } from 'react';

import { cn } from '../../lib/cn';

export interface FieldProps {
  htmlFor?: string;
  label?: string;
  description?: string;
  control: ReactNode;
  stacked?: boolean;
  disabled?: boolean;
  className?: string;
  'data-testid'?: string;
}

/**
 * A labelled control row — label + optional description on one side, the
 * control on the other, or stacked. Generalized out of
 * `settings/controls/SettingsRow`, which now re-exports this.
 *
 * `htmlFor` produces a real `<label>`; without it the label is a `<span>`, so a
 * caller that forgets the association does not get a label pointing nowhere.
 */
const Field = ({
  htmlFor,
  label,
  description,
  control,
  stacked = false,
  disabled = false,
  className,
  'data-testid': testId,
}: FieldProps) => {
  const labelEl =
    label && htmlFor ? (
      <label htmlFor={htmlFor} className="text-sm font-medium text-content">
        {label}
      </label>
    ) : label ? (
      <span className="text-sm font-medium text-content">{label}</span>
    ) : null;

  return (
    <div
      data-slot="field"
      data-testid={testId}
      className={cn(
        stacked
          ? 'flex flex-col gap-2 px-4 py-3'
          : 'flex items-center justify-between gap-4 px-4 py-3',
        disabled && 'pointer-events-none opacity-50',
        className
      )}>
      {(labelEl || description) && (
        <div className={stacked ? undefined : 'min-w-0 flex-1'}>
          {labelEl}
          {description && (
            <p className="mt-0.5 text-xs leading-relaxed text-content-muted">{description}</p>
          )}
        </div>
      )}
      <div className={stacked ? 'w-full' : 'shrink-0'}>{control}</div>
    </div>
  );
};

export default Field;
