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
          <div className="flex items-center gap-2 text-xs text-amber-300">
            <div className="h-2 w-2 rounded-full bg-amber-400 animate-pulse" />
            Unsaved changes
          </div>
        )}
      </div>

      {success && (
        <div className="flex items-center gap-2 rounded-lg border border-sage-500/40 bg-sage-500/10 px-3 py-2 text-sm text-sage-200">
          <CheckIcon className="h-4 w-4" />
          {typeof success === 'string' ? success : 'Operation completed successfully'}
        </div>
      )}

      {error && (
        <div className="flex items-center gap-2 rounded-lg border border-coral-500/40 bg-coral-500/10 px-3 py-2 text-sm text-coral-200">
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
      'bg-primary-600 hover:bg-primary-500 active:bg-primary-700 text-white shadow-soft hover:shadow-lg hover:shadow-primary-500/25 focus:ring-2 focus:ring-primary-500/50 focus:ring-offset-2 focus:ring-offset-black',
    secondary:
      'bg-stone-800 hover:bg-stone-700 active:bg-stone-600 text-white border border-stone-600 focus:ring-2 focus:ring-primary-500/50 focus:ring-offset-2 focus:ring-offset-black',
    outline:
      'border border-stone-600 text-white hover:bg-white/10 active:bg-white/20 focus:ring-2 focus:ring-primary-500/50 focus:ring-offset-2 focus:ring-offset-black',
  };

  return (
    <button
      className={`${baseClasses} ${variantClasses[variant]} ${className} flex items-center justify-center`}
      onClick={onClick}
      disabled={disabled || loading}>
      <div className="flex items-center gap-2">
        {loading && (
          <div className="h-4 w-4 border-2 border-white/20 border-t-white rounded-full animate-spin" />
        )}
        <span>{children}</span>
      </div>
    </button>
  );
};

export default ActionPanel;
export { PrimaryButton };
