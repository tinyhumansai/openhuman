import { useId } from 'react';

import type { FieldRequirement } from '../../types/channels';
import Checkbox from '../ui/Checkbox';
import TextField from '../ui/TextField';

interface ChannelFieldInputProps {
  field: FieldRequirement;
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
}

const ChannelFieldInput = ({ field, value, onChange, disabled }: ChannelFieldInputProps) => {
  const fieldId = useId();

  if (field.field_type === 'boolean') {
    const checked = value === 'true';
    return (
      <label className="flex items-start gap-2" htmlFor={fieldId}>
        <Checkbox
          id={fieldId}
          checked={checked}
          disabled={disabled}
          onCheckedChange={next => onChange(next ? 'true' : 'false')}
          className="mt-0.5"
        />
        <span className="min-w-0">
          <span className="block text-xs font-medium text-content-secondary">
            {field.label}
            {field.required && <span className="ml-0.5 text-coral-500">*</span>}
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
      <label className="mb-1 block text-xs text-content-muted" htmlFor={fieldId}>
        {field.label}
        {field.required && <span className="ml-0.5 text-coral-500">*</span>}
      </label>
      <TextField
        id={fieldId}
        type={field.field_type === 'secret' ? 'password' : 'text'}
        value={value}
        onChange={e => onChange(e.target.value)}
        placeholder={field.placeholder || field.label}
        disabled={disabled}
      />
    </div>
  );
};

export default ChannelFieldInput;
