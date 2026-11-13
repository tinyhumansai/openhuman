import { DEFAULT_INTL_LOCALE } from '../i18n/intlLocale';

/**
 * Format a flow-run ISO timestamp for display: short month/day + time, in
 * the app's active locale. Returns `null` for a missing or unparsable value
 * so callers can render nothing rather than "Invalid Date".
 *
 * Pass `withSeconds: true` for contexts (like a run inspector) precise
 * enough that second-level resolution helps distinguish nearby events.
 *
 * `locale` is a BCP-47 tag — pass `intlLocale(locale)` from `useT()` so the
 * rendering follows the app's active language instead of the host's.
 */
export function formatRunTimestamp(
  value: string | null | undefined,
  options?: { withSeconds?: boolean; locale?: string }
): string | null {
  if (!value) return null;
  const parsed = Date.parse(value);
  if (!Number.isFinite(parsed)) return null;
  return new Intl.DateTimeFormat(options?.locale ?? DEFAULT_INTL_LOCALE, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
    ...(options?.withSeconds ? { second: '2-digit' as const } : {}),
  }).format(new Date(parsed));
}
