import { cloneElement, type ReactElement, type ReactNode } from 'react';

export interface FormFieldProps {
  id: string;
  label: ReactNode;
  children: ReactElement;
  description?: ReactNode;
  error?: ReactNode;
  required?: boolean;
  className?: string;
}

type FormControlProps = {
  id?: string;
  'aria-describedby'?: string;
  'aria-invalid'?: boolean | 'true' | 'false';
  required?: boolean;
};

export default function FormField({
  id,
  label,
  children,
  description,
  error,
  required,
  className,
}: FormFieldProps) {
  const child = children as ReactElement<FormControlProps>;
  const descriptionId = `${id}-description`;
  const errorId = `${id}-error`;
  const describedBy = [description != null ? descriptionId : null, error != null ? errorId : null]
    .filter(Boolean)
    .join(' ');
  const controlId = child.props.id ?? id;
  const control = cloneElement(child, {
    id: controlId,
    'aria-describedby': child.props['aria-describedby'] ?? (describedBy || undefined),
    'aria-invalid': child.props['aria-invalid'] ?? error != null,
    required: child.props.required ?? required,
  });

  return (
    <div className={className}>
      <label htmlFor={controlId} className="mb-1 block text-xs font-medium text-content-secondary">
        {label}
      </label>
      {control}
      {description != null && (
        <p id={descriptionId} className="mt-1 text-xs text-content-muted">
          {description}
        </p>
      )}
      {error != null && (
        <p id={errorId} role="alert" className="mt-1 text-xs text-red-600 dark:text-red-400">
          {error}
        </p>
      )}
    </div>
  );
}
