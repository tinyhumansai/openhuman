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
          className="mt-0.5 h-4 w-4 rounded border-stone-300 text-primary-600 focus:ring-primary-500 disabled:opacity-50"
        />
        <span className="min-w-0">
          <span className="block text-xs font-medium text-stone-700 dark:text-neutral-200">
            {field.label}
            {field.required && <span className="text-coral-500 ml-0.5">*</span>}
          </span>
          {field.placeholder && (
            <span className="block text-[11px] text-stone-500 dark:text-neutral-400">
              {field.placeholder}
            </span>
          )}
        </span>
      </label>
    );
  }

  return (
    <div>
      <label className="block text-xs text-stone-500 dark:text-neutral-400 mb-1">
        {field.label}
        {field.required && <span className="text-coral-500 ml-0.5">*</span>}
      </label>
      <input
        type={field.field_type === 'secret' ? 'password' : 'text'}
        value={value}
        onChange={e => onChange(e.target.value)}
        placeholder={field.placeholder || field.label}
        disabled={disabled}
        className="w-full rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:border-primary-500/60 disabled:opacity-50"
      />
    </div>
  );
};

export default ChannelFieldInput;
