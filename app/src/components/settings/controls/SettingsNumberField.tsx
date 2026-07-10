import Input from '../../ui/Input';

export interface SettingsNumberFieldProps {
  id: string;
  value: string;
  onChange: (v: string) => void;
  onCommit: () => void;
  /** Optional unit suffix shown beside the field (e.g. "seconds"). */
  unit?: string;
  /** Optional bounds — when both are set, a "{min}–{max}" range hint renders. */
  min?: number;
  max?: number;
  /** Step granularity; default 1. Pass a fraction for decimal fields. */
  step?: number;
  disabled?: boolean;
  invalid?: boolean;
  'aria-label': string;
  'data-testid'?: string;
}

const SettingsNumberField = ({
  id,
  value,
  onChange,
  onCommit,
  unit,
  min,
  max,
  step = 1,
  disabled = false,
  invalid = false,
  'aria-label': ariaLabel,
  'data-testid': testId,
}: SettingsNumberFieldProps) => {
  const containerClass = ['flex items-center gap-2', disabled ? 'opacity-50' : '']
    .filter(Boolean)
    .join(' ');

  const hasRange = min !== undefined && max !== undefined;
  const hasMeta = Boolean(unit) || hasRange;

  return (
    <div className={containerClass} data-testid={testId}>
      <Input
        id={id}
        type="number"
        inputMode="numeric"
        inputSize="sm"
        className="w-24"
        value={value}
        min={min}
        max={max}
        step={step}
        disabled={disabled}
        invalid={invalid}
        aria-label={ariaLabel}
        onChange={e => onChange(e.target.value)}
        onBlur={onCommit}
        onKeyDown={e => {
          if (e.key === 'Enter') {
            e.preventDefault();
            onCommit();
          }
        }}
      />
      {hasMeta && (
        <div className="flex flex-col leading-tight">
          {unit && (
            <span className="text-xs font-medium text-content-secondary dark:text-content-muted">
              {unit}
            </span>
          )}
          {hasRange && (
            <span className="text-[11px] text-content-faint">
              {min}&#x2013;{max}
            </span>
          )}
        </div>
      )}
    </div>
  );
};

export default SettingsNumberField;
