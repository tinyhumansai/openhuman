import { type ReactNode } from 'react';

export interface SettingsRowProps {
  htmlFor?: string;
  label?: string;
  description?: string;
  control: ReactNode;
  stacked?: boolean;
  disabled?: boolean;
  className?: string;
  'data-testid'?: string;
}

const SettingsRow = ({
  htmlFor,
  label,
  description,
  control,
  stacked = false,
  disabled = false,
  className,
  'data-testid': testId,
}: SettingsRowProps) => {
  const containerBase = stacked
    ? 'flex flex-col gap-2 px-4 py-3'
    : 'flex items-center justify-between gap-4 px-4 py-3';
  const disabledClass = disabled ? 'opacity-50 pointer-events-none' : '';
  const containerClass = [containerBase, disabledClass, className ?? ''].filter(Boolean).join(' ');

  const labelEl =
    label && htmlFor ? (
      <label htmlFor={htmlFor} className="text-sm font-medium text-content">
        {label}
      </label>
    ) : label ? (
      <span className="text-sm font-medium text-content">{label}</span>
    ) : null;

  const labelBlock =
    labelEl || description ? (
      <div className={stacked ? undefined : 'flex-1 min-w-0'}>
        {labelEl}
        {description && (
          <p className="mt-0.5 text-xs text-content-muted leading-relaxed">{description}</p>
        )}
      </div>
    ) : null;

  const controlWrapper = stacked ? (
    <div className="w-full">{control}</div>
  ) : (
    <div className="flex-shrink-0">{control}</div>
  );

  return (
    <div className={containerClass} data-testid={testId}>
      {labelBlock}
      {controlWrapper}
    </div>
  );
};

export default SettingsRow;
