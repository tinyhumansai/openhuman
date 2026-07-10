/**
 * Colour conversion helpers for the theme system.
 *
 * Theme tokens are stored as space-separated RGB **channel** triples
 * (e.g. `"47 110 244"`) so Tailwind's `rgb(var(--token) / <alpha-value>)`
 * wiring keeps opacity modifiers working. The Theme Studio's native colour
 * inputs speak hex, so we convert at the UI boundary.
 */

/** Clamp to a 0–255 integer. */
function byte(n: number): number {
  if (Number.isNaN(n)) return 0;
  return Math.max(0, Math.min(255, Math.round(n)));
}

/** `"47 110 244"` → `"#2f6ef4"`. Returns `#000000` for malformed input. */
export function channelsToHex(channels: string): string {
  const parts = channels
    .trim()
    .split(/\s+/)
    .map(p => Number(p));
  if (parts.length < 3) return '#000000';
  const [r, g, b] = parts;
  const hex = (v: number) => byte(v).toString(16).padStart(2, '0');
  return `#${hex(r)}${hex(g)}${hex(b)}`;
}

/** `"#2f6ef4"` (or `"#2f6"`) → `"47 110 244"`. Returns `"0 0 0"` if unparseable. */
export function hexToChannels(hex: string): string {
  let h = hex.trim().replace(/^#/, '');
  if (h.length === 3) {
    h = h
      .split('')
      .map(c => c + c)
      .join('');
  }
  if (!/^[0-9a-fA-F]{6}$/.test(h)) return '0 0 0';
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  return `${r} ${g} ${b}`;
}

/** True when a string looks like a valid `"R G B"` channel triple. */
export function isChannelTriple(value: string): boolean {
  const parts = value.trim().split(/\s+/);
  return (
    parts.length === 3 &&
    parts.every(p => /^\d{1,3}$/.test(p) && Number(p) >= 0 && Number(p) <= 255)
  );
}

/**
 * Relative luminance (WCAG) of a channel triple, 0 (black) … 1 (white).
 * Used to warn when a custom theme risks unreadable contrast.
 */
export function channelLuminance(channels: string): number {
  const parts = channels
    .trim()
    .split(/\s+/)
    .map(p => Number(p) / 255);
  if (parts.length < 3) return 0;
  const lin = parts.map(c => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
}
