import React from 'react';

interface InputGroupProps {
  title?: string;
  description?: string;
  children: React.ReactNode;
  className?: string;
}

const InputGroup: React.FC<InputGroupProps> = ({
  title,
  description,
  children,
  className = '',
}) => {
  return (
    <div className={`space-y-6 ${className}`}>
      {title && (
        <div className="space-y-2">
          <h4 className="text-lg font-medium text-stone-900">{title}</h4>
          {description && <p className="text-sm text-stone-500">{description}</p>}
        </div>
      )}

      <div className="rounded-lg border border-stone-200 bg-stone-50 backdrop-blur-sm p-6">
        <div className="grid gap-6 md:grid-cols-2">{children}</div>
      </div>
    </div>
  );
};

interface FieldProps {
  label: string;
  children: React.ReactNode;
  helpText?: string;
  className?: string;
  fullWidth?: boolean;
}

const Field: React.FC<FieldProps> = ({
  label,
  children,
  helpText,
  className = '',
  fullWidth = false,
}) => {
  return (
    <label
      className={`space-y-3 text-sm text-stone-600 ${fullWidth ? 'md:col-span-2' : ''} ${className}`}>
      <div>
        <span className="font-medium">{label}</span>
        {helpText && <p className="text-xs text-stone-500 leading-relaxed mt-1">{helpText}</p>}
      </div>
      {children}
    </label>
  );
};

interface CheckboxFieldProps {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  helpText?: string;
  className?: string;
}

const CheckboxField: React.FC<CheckboxFieldProps> = ({
  label,
  checked,
  onChange,
  helpText,
  className = '',
}) => {
  return (
    <div className={`flex flex-col gap-3 ${className}`}>
      <label className="flex items-center gap-3 text-sm text-stone-600 cursor-pointer">
        <input
          type="checkbox"
          className="w-5 h-5 rounded border-2 border-stone-300 bg-white text-primary-500 focus:ring-2 focus:ring-primary-500/30 focus:border-primary-500/50 transition-all duration-200"
          checked={checked}
          onChange={e => onChange(e.target.checked)}
        />
        <span className="font-medium">{label}</span>
      </label>
      {helpText && <p className="text-xs text-stone-500 ml-7 leading-relaxed">{helpText}</p>}
    </div>
  );
};

export default InputGroup;
export { Field, CheckboxField };
