import { CheckIcon, ExclamationTriangleIcon } from '@heroicons/react/24/outline';
import React from 'react';

interface ActionPanelProps {
  children: React.ReactNode;
  hasChanges?: boolean;
  success?: boolean | string;
  error?: string;
  className?: string;
}

const ActionPanel: React.FC<ActionPanelProps> = ({
  children,
  hasChanges = false,
  success = false,
  error,
  className = '',
}) => {
  return (
    <div className={`space-y-6 ${className}`}>
      <div className="flex flex-wrap items-center gap-4">
        {children}
        {hasChanges && (
          <div className="flex items-center gap-2 text-xs text-amber-700">
            <div className="h-2 w-2 rounded-full bg-amber-400 animate-pulse" />
            Unsaved changes
          </div>
        )}
      </div>

      {success && (
        <div className="flex items-center gap-2 rounded-lg border border-sage-500/40 bg-sage-50 px-3 py-2 text-sm text-sage-700">
          <CheckIcon className="h-4 w-4" />
          {typeof success === 'string' ? success : 'Operation completed successfully'}
        </div>
      )}

      {error && (
        <div className="flex items-center gap-2 rounded-lg border border-coral-500/40 bg-coral-50 px-3 py-2 text-sm text-coral-700">
          <ExclamationTriangleIcon className="h-4 w-4" />
          {error}
        </div>
      )}
    </div>
  );
};

interface PrimaryButtonProps {
  onClick: () => void;
  loading?: boolean;
  disabled?: boolean;
  variant?: 'primary' | 'secondary' | 'outline';
  children: React.ReactNode;
  className?: string;
}

const PrimaryButton: React.FC<PrimaryButtonProps> = ({
  onClick,
  loading = false,
  disabled = false,
  variant = 'primary',
  children,
  className = '',
}) => {
  const baseClasses =
    'px-6 py-3 rounded-lg font-medium transition-all duration-200 focus:outline-none disabled:opacity-50 disabled:cursor-not-allowed';
  const variantClasses = {
    primary:
      'bg-primary-600 hover:bg-primary-500 active:bg-primary-700 text-white shadow-soft hover:shadow-lg hover:shadow-primary-500/25 focus:ring-2 focus:ring-primary-500/50 focus:ring-offset-2 focus:ring-offset-white',
    secondary:
      'bg-stone-100 hover:bg-stone-200 active:bg-stone-300 text-stone-900 border border-stone-200 focus:ring-2 focus:ring-primary-500/50 focus:ring-offset-2 focus:ring-offset-white',
    outline:
      'border border-stone-200 text-stone-900 hover:bg-stone-100 active:bg-stone-200 focus:ring-2 focus:ring-primary-500/50 focus:ring-offset-2 focus:ring-offset-white',
  };

  return (
    <button
      className={`${baseClasses} ${variantClasses[variant]} ${className} flex items-center justify-center`}
      onClick={onClick}
      disabled={disabled || loading}>
      <div className="flex items-center gap-2">
        {loading && (
          <div className="h-4 w-4 border-2 border-white/40 border-t-white rounded-full animate-spin" />
        )}
        <span>{children}</span>
      </div>
    </button>
  );
};

export default ActionPanel;
export { PrimaryButton };
