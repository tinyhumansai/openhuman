import { SelectContent, SelectItem, SelectRoot, SelectTrigger, SelectValue } from '../ui';

interface FilterOption {
  value: string;
  label: string;
}

interface FeedbackFilterSelectProps {
  value: string;
  options: FilterOption[];
  onChange: (value: string) => void;
  /** Accessible label for the trigger button. */
  ariaLabel: string;
}

/**
 * Styled dropdown for the board filters. A short (<= ~20 option), single-line
 * list that needs no custom item rendering beyond Radix's built-in selected
 * checkmark — the case `Select.tsx`'s doc comment calls out as the right fit
 * for Radix `Select` over `NativeSelect`. Radix owns the listbox ARIA
 * contract (roving focus, typeahead, Home/End, Enter/Space, Escape,
 * outside-click dismissal, focus return to the trigger) so this component
 * only wires options to items.
 */
export default function FeedbackFilterSelect({
  value,
  options,
  onChange,
  ariaLabel,
}: FeedbackFilterSelectProps) {
  return (
    <SelectRoot value={value} onValueChange={onChange}>
      <SelectTrigger
        aria-label={ariaLabel}
        inputSize="sm"
        className="w-auto gap-1.5 text-xs font-medium">
        <SelectValue />
      </SelectTrigger>
      <SelectContent align="end">
        {options.map(option => (
          <SelectItem key={option.value} value={option.value}>
            {option.label}
          </SelectItem>
        ))}
      </SelectContent>
    </SelectRoot>
  );
}
