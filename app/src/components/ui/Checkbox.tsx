import { useEffect, useRef } from 'react';

import { cn } from '../../lib/cn';

export interface CheckboxProps {
  id?: string;
  checked: boolean;
  onCheckedChange: (next: boolean) => void;
  disabled?: boolean;
  /** Tri-state. Set imperatively — the DOM has no `indeterminate` attribute. */
  indeterminate?: boolean;
  className?: string;
  'aria-label'?: string;
  'data-testid'?: string;
}

/**
 * A NATIVE `<input type="checkbox">`, deliberately — this is the one control in
 * the set that Radix should not own.
 *
 * Radix renders a checkbox as `<button role="checkbox">` plus a visually hidden
 * input that carries no id. That is fine for a11y but it silently breaks
 * everything that treats a checkbox as an input: WebDriver's `isSelected()`
 * returns false forever, `getElementById(...).checked` becomes `undefined`, and
 * the DOM fallback in `test/e2e/specs/chat-harness-wallet-flow.spec.ts` — which
 * sets `.checked` on `#mnemonic-confirm-checkbox` to get past the recovery
 * phrase consent gate — becomes a no-op. That spec runs only in CI Full, so
 * the breakage would not have surfaced on the fast lane.
 *
 * A native checkbox is already accessible and already participates in forms.
 * There was no defect here to fix, so the Radix version bought styling
 * consistency at the cost of a working flow. The only real bug in the original
 * — a focus ring offset against a hardcoded white, which ignores user themes —
 * is fixed below.
 */
const Checkbox = ({
  id,
  checked,
  onCheckedChange,
  disabled = false,
  indeterminate = false,
  className,
  'aria-label': ariaLabel,
  'data-testid': testId,
}: CheckboxProps) => {
  const innerRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (innerRef.current) innerRef.current.indeterminate = indeterminate;
  }, [indeterminate]);

  return (
    <input
      ref={innerRef}
      id={id}
      type="checkbox"
      data-slot="checkbox"
      checked={checked}
      disabled={disabled}
      aria-label={ariaLabel}
      data-testid={testId}
      onChange={e => onCheckedChange(e.target.checked)}
      className={cn(
        'h-4 w-4 cursor-pointer rounded-sm border border-line-strong bg-surface accent-primary-500',
        // The forms plugin draws a white check/dash; bg-surface otherwise
        // overrides its selected background and hides the mark in light mode.
        // Keep the dark surface: some dark themes have light primary accents
        // that do not contrast with the plugin's white mark.
        'checked:bg-primary-500 indeterminate:bg-primary-500 dark:checked:bg-surface dark:indeterminate:bg-surface',
        'transition-colors duration-150',
        'focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500 focus-visible:ring-offset-1',
        // Offsets against the themed surface. The original used
        // `ring-offset-white` plus a hardcoded dark companion, which stayed
        // white under any custom theme.
        'focus-visible:ring-offset-surface',
        'disabled:cursor-not-allowed disabled:opacity-50',
        className
      )}
    />
  );
};

export default Checkbox;
