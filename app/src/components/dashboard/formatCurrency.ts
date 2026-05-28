/**
 * Format a USD-denominated amount with the requested display currency label.
 * Falls back to the locale "USD" formatter when the configured currency
 * code is not a valid ISO-4217 currency Intl supports — this keeps
 * arbitrary display labels (e.g. "USD ($)") from throwing.
 */
export function formatCurrency(amountUsd: number, currency: string): string {
  const safe = Number.isFinite(amountUsd) ? amountUsd : 0;
  const normalized = currency?.trim() ? currency.trim().toUpperCase() : 'USD';
  try {
    return new Intl.NumberFormat(undefined, {
      style: 'currency',
      currency: normalized,
      maximumFractionDigits: safe >= 100 ? 0 : 2,
    }).format(safe);
  } catch {
    return new Intl.NumberFormat(undefined, {
      style: 'currency',
      currency: 'USD',
      maximumFractionDigits: safe >= 100 ? 0 : 2,
    }).format(safe);
  }
}

export function formatTokens(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '0';
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return Math.round(n).toString();
}

export function shortDayLabel(isoDate: string): string {
  try {
    const date = new Date(`${isoDate}T00:00:00Z`);
    return new Intl.DateTimeFormat(undefined, { weekday: 'short', timeZone: 'UTC' }).format(date);
  } catch {
    return isoDate.slice(5);
  }
}

/**
 * Full long-form date label for tooltips (e.g. "Wed, May 27").
 * Falls back to the raw ISO string when parsing fails.
 */
export function longDateLabel(isoDate: string): string {
  try {
    const date = new Date(`${isoDate}T00:00:00Z`);
    return new Intl.DateTimeFormat(undefined, {
      weekday: 'short',
      month: 'short',
      day: 'numeric',
      timeZone: 'UTC',
    }).format(date);
  } catch {
    return isoDate;
  }
}

/**
 * Day-of-month number for the X-axis sub-line, in UTC. Returns the
 * trailing two digits of the ISO date on any parse failure.
 */
export function dayOfMonth(isoDate: string): string {
  try {
    const date = new Date(`${isoDate}T00:00:00Z`);
    return new Intl.DateTimeFormat(undefined, { day: 'numeric', timeZone: 'UTC' }).format(date);
  } catch {
    return isoDate.slice(-2);
  }
}

/**
 * i18n translator signature accepted by [`relativeTime`] — matches the
 * shape of `useT()`'s `t` so callers can pass it directly. The function
 * is invoked with a string key and is expected to return the localised
 * value (or the key itself when no translation is available).
 */
export type RelativeTimeTranslator = (key: string, fallback?: string) => string;

/**
 * Human-friendly relative-time string for the "updated Ns ago" pill.
 *
 * Localised via the i18n translator the caller hands in — never returns
 * hard-coded English. Keys consumed (all under `settings.costDashboard.*`):
 * `justNow`, `secondsAgo`, `minutesAgo`, `hoursAgo`, `daysAgo`. The
 * three numeric variants use a literal `{value}` placeholder so locales
 * can position the number naturally; the placeholder is substituted
 * verbatim before return.
 *
 * Caps at 60s/60m/24h boundaries; returns the localised "Just now"
 * within 5s to avoid a flickering "0s ago" right after a refetch.
 */
export function relativeTime(
  timestampMs: number,
  t: RelativeTimeTranslator,
  nowMs: number = Date.now()
): string {
  const deltaSec = Math.max(0, Math.floor((nowMs - timestampMs) / 1000));
  if (deltaSec < 5) return t('settings.costDashboard.justNow');
  if (deltaSec < 60) {
    return t('settings.costDashboard.secondsAgo').replaceAll('{value}', String(deltaSec));
  }
  const deltaMin = Math.floor(deltaSec / 60);
  if (deltaMin < 60) {
    return t('settings.costDashboard.minutesAgo').replaceAll('{value}', String(deltaMin));
  }
  const deltaHr = Math.floor(deltaMin / 60);
  if (deltaHr < 24) {
    return t('settings.costDashboard.hoursAgo').replaceAll('{value}', String(deltaHr));
  }
  const deltaDay = Math.floor(deltaHr / 24);
  return t('settings.costDashboard.daysAgo').replaceAll('{value}', String(deltaDay));
}
