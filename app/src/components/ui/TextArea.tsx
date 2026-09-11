import { forwardRef, type TextareaHTMLAttributes } from 'react';

import { cn } from '../../lib/cn';

export interface TextAreaProps extends TextareaHTMLAttributes<HTMLTextAreaElement> {
  invalid?: boolean;
}

const TextArea = forwardRef<HTMLTextAreaElement, TextAreaProps>(
  ({ invalid = false, className, ...rest }, ref) => (
    <textarea
      ref={ref}
      data-slot="textarea"
      aria-invalid={invalid || undefined}
      className={cn(
        'block w-full rounded-lg border bg-surface px-3 py-2 text-sm text-content placeholder-content-faint',
        'transition-colors duration-150 focus:outline-hidden focus:ring-2',
        'disabled:opacity-50',
        invalid
          ? 'border-coral-400 focus:border-coral-500 focus:ring-coral-500/20'
          : 'border-line-strong focus:border-primary-500 focus:ring-primary-500/20',
        className
      )}
      {...rest}
    />
  )
);
TextArea.displayName = 'TextArea';

export default TextArea;
