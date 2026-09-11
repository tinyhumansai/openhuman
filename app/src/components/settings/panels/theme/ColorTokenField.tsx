import { useId } from 'react';

import { channelsToHex, hexToChannels } from '../../../../lib/theme/color';

interface ColorTokenFieldProps {
  /** Token key without `--` (e.g. `surface-canvas`). */
  tokenKey: string;
  /** Human label for the swatch row. */
  label: string;
  /** Current value as a `"R G B"` channel triple (effective or override). */
  value: string;
  /** Whether this field is editable (false when a built-in theme is active). */
  disabled?: boolean;
  /** Called with the new `"R G B"` channel triple. */
  onChange: (channels: string) => void;
}

/**
 * A single editable colour token row: the swatch, then what it is.
 *
 * It used to be a `Field`, whose row is `justify-between` — so the name sat on
 * the far left and a 48px swatch on the far right, with the whole panel width
 * between them. The swatch is the content of this row and it was both the
 * smallest thing in it and the furthest from its own label. Leading the row
 * with it puts the colour next to the name it belongs to, and the row no longer
 * cares how wide the panel is.
 */
const ColorTokenField = ({ tokenKey, label, value, disabled, onChange }: ColorTokenFieldProps) => {
  const id = useId();
  const hex = channelsToHex(value);

  return (
    <label
      htmlFor={id}
      className={`flex items-center gap-3 py-2 ${disabled ? 'cursor-not-allowed' : 'cursor-pointer'}`}>
      <input
        id={id}
        type="color"
        value={hex}
        disabled={disabled}
        onChange={e => onChange(hexToChannels(e.target.value))}
        aria-label={label}
        className="h-9 w-9 shrink-0 cursor-pointer rounded-lg border border-line bg-surface p-0 disabled:cursor-not-allowed disabled:opacity-50"
      />
      <span className="flex min-w-0 flex-1 items-baseline gap-2">
        <span className="truncate text-sm text-content">{label}</span>
        {/* The hex is the value you are about to change, so it stays on the
            same line as the name rather than dropping to a sub-label. */}
        <span className="shrink-0 font-mono text-xs tabular-nums text-content-muted">{hex}</span>
        <span className="truncate font-mono text-[11px] text-content-faint">--{tokenKey}</span>
      </span>
    </label>
  );
};

export default ColorTokenField;
