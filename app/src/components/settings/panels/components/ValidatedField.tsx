import { CheckCircleIcon, ExclamationTriangleIcon } from '@heroicons/react/24/outline';
import React from 'react';

interface ValidatedFieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  error?: string;
  required?: boolean;
  type?: 'text' | 'password' | 'url' | 'number';
  placeholder?: string;
  helpText?: string;
  className?: string;
  fullWidth?: boolean;
  validation?: 'valid' | 'invalid' | 'none';
  disabled?: boolean;
}

const ValidatedField: React.FC<ValidatedFieldProps> = ({
  label,
  value,
  onChange,
  error,
  required = false,
  type = 'text',
  placeholder,
  helpText,
  className = '',
  fullWidth = false,
  validation = 'none',
  disabled = false,
}) => {
  const hasError = !!error;
  const isValid = validation === 'valid' && !hasError && value.trim() !== '';

  const inputClasses = `
    w-full px-4 py-3 rounded-lg border transition-all duration-200
    focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-offset-white
    disabled:opacity-50 disabled:cursor-not-allowed
    ${
      hasError
        ? 'border-coral-500/60 bg-coral-50 text-coral-700 placeholder-coral-400/50 focus:border-coral-500 focus:ring-coral-500/30'
        : isValid
          ? 'border-sage-500/60 bg-sage-50 text-stone-900 placeholder-stone-400 focus:border-sage-500 focus:ring-sage-500/30'
          : 'border-stone-200 bg-white text-stone-900 placeholder-stone-400 focus:border-primary-500/50 focus:ring-primary-500/30'
    }
  `;

  return (
    <label
      className={`space-y-3 text-sm text-stone-600 ${fullWidth ? 'md:col-span-2' : ''} ${className}`}>
      <div>
        <span className="font-medium">
          {label}
          {required && <span className="text-coral-400 ml-1">*</span>}
        </span>
        {helpText && <p className="text-xs text-stone-500 leading-relaxed mt-1">{helpText}</p>}
      </div>

      <div className="relative">
        <input
          type={type}
          className={inputClasses}
          placeholder={placeholder}
          value={value}
          onChange={e => onChange(e.target.value)}
          disabled={disabled}
        />

        {/* Validation icon */}
        {(hasError || isValid) && (
          <div className="absolute inset-y-0 right-0 flex items-center pr-3 pointer-events-none">
            {hasError ? (
              <ExclamationTriangleIcon className="h-5 w-5 text-coral-400" />
            ) : isValid ? (
              <CheckCircleIcon className="h-5 w-5 text-sage-400" />
            ) : null}
          </div>
        )}
      </div>

      {/* Error message */}
      {hasError && (
        <div className="flex items-center gap-2 text-xs text-coral-600">
          <ExclamationTriangleIcon className="h-3 w-3 flex-shrink-0" />
          <span>{error}</span>
        </div>
      )}
    </label>
  );
};

interface ValidatedSelectProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  options: Array<{ value: string; label: string; description?: string }>;
  error?: string;
  required?: boolean;
  placeholder?: string;
  helpText?: string;
  className?: string;
  fullWidth?: boolean;
  disabled?: boolean;
  validation?: 'valid' | 'invalid' | 'none';
}

const ValidatedSelect: React.FC<ValidatedSelectProps> = ({
  label,
  value,
  onChange,
  options,
  error,
  required = false,
  placeholder = 'Select option...',
  helpText,
  className = '',
  fullWidth = false,
  disabled = false,
  validation = 'none',
}) => {
  const hasError = !!error;
  const isValid = validation === 'valid' && !hasError && value.trim() !== '';

  const selectClasses = `
    w-full px-4 py-3 rounded-lg border transition-all duration-200
    focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-offset-white
    disabled:opacity-50 disabled:cursor-not-allowed
    ${
      hasError
        ? 'border-coral-500/60 bg-coral-50 text-coral-700 focus:border-coral-500 focus:ring-coral-500/30'
        : isValid
          ? 'border-sage-500/60 bg-sage-50 text-stone-900 focus:border-sage-500 focus:ring-sage-500/30'
          : 'border-stone-200 bg-white text-stone-900 focus:border-primary-500/50 focus:ring-primary-500/30'
    }
  `;

  return (
    <label
      className={`space-y-3 text-sm text-stone-600 ${fullWidth ? 'md:col-span-2' : ''} ${className}`}>
      <div>
        <span className="font-medium">
          {label}
          {required && <span className="text-coral-400 ml-1">*</span>}
        </span>
        {helpText && <p className="text-xs text-stone-500 leading-relaxed mt-1">{helpText}</p>}
      </div>

      <div className="relative">
        <select
          className={selectClasses}
          value={value}
          onChange={e => onChange(e.target.value)}
          disabled={disabled}>
          {placeholder && <option value="">{placeholder}</option>}
          {options.map(option => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>

        {/* Validation icon */}
        {(hasError || isValid) && (
          <div className="absolute inset-y-0 right-8 flex items-center pr-3 pointer-events-none">
            {hasError ? (
              <ExclamationTriangleIcon className="h-5 w-5 text-coral-400" />
            ) : isValid ? (
              <CheckCircleIcon className="h-5 w-5 text-sage-400" />
            ) : null}
          </div>
        )}
      </div>

      {/* Error message */}
      {hasError && (
        <div className="flex items-center gap-2 text-xs text-coral-600">
          <ExclamationTriangleIcon className="h-3 w-3 flex-shrink-0" />
          <span>{error}</span>
        </div>
      )}
    </label>
  );
};

export default ValidatedField;
export { ValidatedSelect };
