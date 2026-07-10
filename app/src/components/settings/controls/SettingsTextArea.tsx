import { forwardRef, type TextareaHTMLAttributes } from 'react';

export interface SettingsTextAreaProps extends TextareaHTMLAttributes<HTMLTextAreaElement> {
  invalid?: boolean;
  rows?: number;
}

const SettingsTextArea = forwardRef<HTMLTextAreaElement, SettingsTextAreaProps>(
  ({ invalid = false, className, ...rest }, ref) => {
    const ringClass = invalid
      ? 'border-coral-400 focus:border-coral-500 focus:ring-coral-500/20'
      : 'border-line-strong focus:border-primary-500 focus:ring-primary-500/20';

    const classes = [
      'block w-full rounded-lg border',
      'bg-surface',
      'text-content',
      'placeholder-content-faint',
      'text-sm px-3 py-2',
      'focus:outline-none focus:ring-2',
      'transition-colors duration-150',
      'disabled:opacity-50',
      ringClass,
      className ?? '',
    ]
      .filter(Boolean)
      .join(' ');

    return <textarea ref={ref} className={classes} {...rest} />;
  }
);
SettingsTextArea.displayName = 'SettingsTextArea';

export default SettingsTextArea;
