/**
 * Truncate `text` to at most `max` characters, appending an ellipsis (`…`)
 * when it was cut. The ellipsis itself counts against `max` UTF-16 code units,
 * so the result is never longer than `max` units and never cuts a surrogate
 * pair in half. Combining sequences may still be split at the boundary.
 * Returns `''` when `max < 1`.
 *
 * Pass `trim: true` to trim whitespace before measuring/truncating (useful
 * for freeform text pulled from tool output or user input).
 */
export function truncateText(text: string, max: number, options?: { trim?: boolean }): string {
  const value = options?.trim ? text.trim() : text;
  if (max < 1) return '';
  if (!(value.length > max)) return value;

  let end = Math.floor(max - 1);
  const before = value.charCodeAt(end - 1);
  const after = value.charCodeAt(end);
  if (before >= 0xd800 && before <= 0xdbff && after >= 0xdc00 && after <= 0xdfff) end--;
  return `${value.slice(0, end)}…`;
}
