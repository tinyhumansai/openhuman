import type { FieldRequirement } from '../../types/channels';

interface ChannelFieldInputProps {
  field: FieldRequirement;
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
}

const ChannelFieldInput = ({ field, value, onChange, disabled }: ChannelFieldInputProps) => {
  return (
    <div>
      <label className="block text-xs text-stone-400 mb-1">
        {field.label}
        {field.required && <span className="text-coral-400 ml-0.5">*</span>}
      </label>
      <input
        type={field.field_type === 'secret' ? 'password' : 'text'}
        value={value}
        onChange={e => onChange(e.target.value)}
        placeholder={field.placeholder || field.label}
        disabled={disabled}
        className="w-full rounded-lg border border-stone-700 bg-stone-900 px-3 py-2 text-sm text-white placeholder:text-stone-500 focus:outline-none focus:border-primary-500/60 disabled:opacity-50"
      />
    </div>
  );
};

export default ChannelFieldInput;
