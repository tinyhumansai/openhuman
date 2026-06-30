import type { FieldRequirement } from '../../types/channels';

interface ChannelFieldInputProps {
  field: FieldRequirement;
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
}

const ChannelFieldInput = ({ field, value, onChange, disabled }: ChannelFieldInputProps) => {
  if (field.field_type === 'boolean') {
    const checked = value === 'true';
    return (
      <label className="flex items-start gap-2">
        <input
          type="checkbox"
          checked={checked}
          disabled={disabled}
          onChange={e => onChange(e.target.checked ? 'true' : 'false')}
          className="mt-0.5 h-4 w-4 rounded border-line-strong text-primary-600 focus:ring-primary-500 disabled:opacity-50"
        />
        <span className="min-w-0">
          <span className="block text-xs font-medium text-content-secondary">
            {field.label}
            {field.required && <span className="text-coral-500 ml-0.5">*</span>}
          </span>
          {field.placeholder && (
            <span className="block text-[11px] text-content-muted">{field.placeholder}</span>
          )}
        </span>
      </label>
    );
  }

  return (
    <div>
      <label className="block text-xs text-content-muted mb-1">
        {field.label}
        {field.required && <span className="text-coral-500 ml-0.5">*</span>}
      </label>
      <input
        type={field.field_type === 'secret' ? 'password' : 'text'}
        value={value}
        onChange={e => onChange(e.target.value)}
        placeholder={field.placeholder || field.label}
        disabled={disabled}
        className="w-full rounded-lg border border-line bg-surface px-3 py-2 text-sm text-content placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:border-primary-500/60 disabled:opacity-50"
      />
    </div>
  );
};

export default ChannelFieldInput;
