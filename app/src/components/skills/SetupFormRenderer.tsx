/**
 * Dynamic form renderer for skill setup steps.
 * Takes a SetupStep and renders appropriate form fields.
 */

import { useState } from "react";
import type { SetupStep, SetupFieldError } from "../../lib/skills/types";

interface SetupFormRendererProps {
  step: SetupStep;
  errors?: SetupFieldError[] | null;
  loading?: boolean;
  onSubmit: (values: Record<string, unknown>) => void;
  onCancel: () => void;
}

export default function SetupFormRenderer({
  step,
  errors,
  loading,
  onSubmit,
  onCancel,
}: SetupFormRendererProps) {
  const [values, setValues] = useState<Record<string, unknown>>(() => {
    const defaults: Record<string, unknown> = {};
    for (const field of step.fields) {
      if (field.default !== undefined && field.default !== null) {
        defaults[field.name] = field.default;
      } else if (field.type === "boolean") {
        defaults[field.name] = false;
      } else {
        defaults[field.name] = "";
      }
    }
    return defaults;
  });

  const [showPasswords, setShowPasswords] = useState<Record<string, boolean>>(
    {},
  );

  const getFieldError = (fieldName: string): string | undefined => {
    return errors?.find((e) => e.field === fieldName)?.message;
  };

  const getGeneralError = (): string | undefined => {
    return errors?.find((e) => e.field === "")?.message;
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    onSubmit(values);
  };

  const updateValue = (name: string, value: unknown) => {
    setValues((prev) => ({ ...prev, [name]: value }));
  };

  return (
    <form onSubmit={handleSubmit} className="space-y-4">
      {/* Step header */}
      <div className="mb-4">
        <h3 className="text-lg font-semibold text-stone-900">{step.title}</h3>
        {step.description && (
          <p className="text-sm text-stone-400 mt-1">{step.description}</p>
        )}
      </div>

      {/* General error */}
      {getGeneralError() && (
        <div className="p-3 bg-coral-500/10 border border-coral-500/30 rounded-xl">
          <p className="text-sm text-coral-400">{getGeneralError()}</p>
        </div>
      )}

      {/* Form fields */}
      {step.fields.map((field) => {
        const fieldError = getFieldError(field.name);
        const value = values[field.name];

        return (
          <div key={field.name} className="space-y-1.5">
            <label className="block text-sm font-medium text-stone-700">
              {field.label}
              {field.required && (
                <span className="text-coral-400 ml-0.5">*</span>
              )}
            </label>

            {field.description && (
              <p className="text-xs text-stone-500">{field.description}</p>
            )}

            {/* Text / Number input */}
            {(field.type === "text" || field.type === "number") && (
              <input
                type={field.type === "number" ? "number" : "text"}
                value={String(value ?? "")}
                onChange={(e) =>
                  updateValue(
                    field.name,
                    field.type === "number"
                      ? Number(e.target.value)
                      : e.target.value,
                  )
                }
                placeholder={field.placeholder ?? undefined}
                disabled={loading}
                className={`w-full px-3 py-2.5 bg-stone-50 border rounded-xl text-sm text-stone-900 placeholder-stone-400 focus:outline-none focus:ring-1 transition-colors ${
                  fieldError
                    ? "border-coral-500/50 focus:ring-coral-500/30"
                    : "border-stone-200 focus:ring-primary-500/30 focus:border-primary-500/50"
                }`}
              />
            )}

            {/* Password input */}
            {field.type === "password" && (
              <div className="relative">
                <input
                  type={showPasswords[field.name] ? "text" : "password"}
                  value={String(value ?? "")}
                  onChange={(e) => updateValue(field.name, e.target.value)}
                  placeholder={field.placeholder ?? undefined}
                  disabled={loading}
                  className={`w-full px-3 py-2.5 pr-10 bg-stone-50 border rounded-xl text-sm text-stone-900 placeholder-stone-400 focus:outline-none focus:ring-1 transition-colors ${
                    fieldError
                      ? "border-coral-500/50 focus:ring-coral-500/30"
                      : "border-stone-200 focus:ring-primary-500/30 focus:border-primary-500/50"
                  }`}
                />
                <button
                  type="button"
                  onClick={() =>
                    setShowPasswords((prev) => ({
                      ...prev,
                      [field.name]: !prev[field.name],
                    }))
                  }
                  className="absolute right-3 top-1/2 -translate-y-1/2 text-stone-500 hover:text-stone-700"
                >
                  {showPasswords[field.name] ? (
                    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.878 9.878L3 3m6.878 6.879L21 21" />
                    </svg>
                  ) : (
                    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
                    </svg>
                  )}
                </button>
              </div>
            )}

            {/* Select dropdown */}
            {field.type === "select" && (
              <select
                value={String(value ?? "")}
                onChange={(e) => updateValue(field.name, e.target.value)}
                disabled={loading}
                className={`w-full px-3 py-2.5 bg-stone-50 border rounded-xl text-sm text-stone-900 focus:outline-none focus:ring-1 transition-colors ${
                  fieldError
                    ? "border-coral-500/50 focus:ring-coral-500/30"
                    : "border-stone-200 focus:ring-primary-500/30 focus:border-primary-500/50"
                }`}
              >
                <option value="">Select...</option>
                {field.options?.map((opt) => (
                  <option key={opt.value} value={opt.value}>
                    {opt.label}
                  </option>
                ))}
              </select>
            )}

            {/* Boolean toggle */}
            {field.type === "boolean" && (
              <label className="flex items-center space-x-3 cursor-pointer">
                <div
                  role="switch"
                  aria-checked={Boolean(value)}
                  onClick={() =>
                    !loading && updateValue(field.name, !value)
                  }
                  className={`relative w-10 h-5 rounded-full transition-colors ${
                    value ? "bg-primary-500" : "bg-stone-600"
                  }`}
                >
                  <div
                    className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform ${
                      value ? "translate-x-5" : "translate-x-0"
                    }`}
                  />
                </div>
                <span className="text-sm text-stone-700">
                  {value ? "Enabled" : "Disabled"}
                </span>
              </label>
            )}

            {/* Field error */}
            {fieldError && (
              <p className="text-xs text-coral-400 mt-1">{fieldError}</p>
            )}
          </div>
        );
      })}

      {/* Actions */}
      <div className="flex space-x-3 pt-2">
        <button
          type="button"
          onClick={onCancel}
          disabled={loading}
          className="flex-1 py-2.5 text-sm font-medium text-stone-600 bg-stone-100 border border-stone-200 rounded-xl hover:bg-stone-200 transition-colors disabled:opacity-50"
        >
          Cancel
        </button>
        <button
          type="submit"
          disabled={loading}
          className="flex-1 py-2.5 text-sm font-medium text-white bg-primary-500 rounded-xl hover:bg-primary-600 transition-colors disabled:opacity-50 flex items-center justify-center"
        >
          {loading ? (
            <svg
              className="animate-spin h-4 w-4 text-white"
              xmlns="http://www.w3.org/2000/svg"
              fill="none"
              viewBox="0 0 24 24"
            >
              <circle
                className="opacity-25"
                cx="12"
                cy="12"
                r="10"
                stroke="currentColor"
                strokeWidth="4"
              />
              <path
                className="opacity-75"
                fill="currentColor"
                d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
              />
            </svg>
          ) : (
            "Continue"
          )}
        </button>
      </div>
    </form>
  );
}
