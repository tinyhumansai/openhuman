import { type ChangeEvent } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { TextField } from '../ui';
import type { ToolkitRequiredField } from './toolkitRequiredFields';

interface RequiredFieldsFormProps {
  fields: readonly ToolkitRequiredField[];
  values: Record<string, string>;
  errors: Record<string, string>;
  onChange: (key: string, value: string) => void;
  /** Autofocus the first input on mount (used by the `needs-fields` recovery phase). */
  autoFocusFirst?: boolean;
}

/**
 * Generic renderer for provider-specific required fields declared in
 * `toolkitRequiredFields.ts`. Replaces the per-toolkit
 * `AtlassianSubdomainInput` / `WabaIdInput` blocks (#2127). Each field
 * shows a label, optional fixed suffix inside the input
 * (e.g. `.atlassian.net`), an optional hint, and an inline error message
 * driven by the `errors` map (keyed by field key, value is an i18n key).
 */
export function RequiredFieldsForm({
  fields,
  values,
  errors,
  onChange,
  autoFocusFirst,
}: RequiredFieldsFormProps) {
  const { t } = useT();
  if (fields.length === 0) return null;
  return (
    <>
      {fields.map((field, idx) => {
        const inputId = `composio-required-${field.key}`;
        const hintId = `${inputId}-hint`;
        const value = values[field.key] ?? '';
        const errorKey = errors[field.key];
        const errorText = errorKey ? t(errorKey) : null;
        return (
          <div key={field.key} className="space-y-1.5">
            <label htmlFor={inputId} className="block text-xs font-medium text-content-secondary">
              {t(field.labelKey)}
              <span className="ml-1 text-coral-500">*</span>
            </label>
            <div className="flex items-center rounded-xl border border-line bg-surface focus-within:border-primary-400 focus-within:ring-2 focus-within:ring-primary-100 overflow-hidden">
              <TextField
                id={inputId}
                data-testid={inputId}
                type="text"
                value={value}
                autoFocus={autoFocusFirst && idx === 0}
                onChange={(e: ChangeEvent<HTMLInputElement>) => onChange(field.key, e.target.value)}
                placeholder={field.placeholderKey ? t(field.placeholderKey) : undefined}
                aria-describedby={hintId}
                invalid={!!errorText}
                className="flex-1 min-w-0 border-0 bg-transparent focus:ring-0"
              />
              {field.suffix && (
                <span className="pr-3 text-xs text-content-faint select-none whitespace-nowrap">
                  {field.suffix}
                </span>
              )}
            </div>
            {/* Always render the hint paragraph with the same id so
                aria-describedby resolves regardless of error state. */}
            {errorText ? (
              <p id={hintId} role="alert" className="text-[11px] text-coral-600">
                {errorText}
              </p>
            ) : (
              field.hintKey && (
                <p id={hintId} className="text-[11px] leading-relaxed text-content-faint">
                  {t(field.hintKey)}
                </p>
              )
            )}
          </div>
        );
      })}
    </>
  );
}
