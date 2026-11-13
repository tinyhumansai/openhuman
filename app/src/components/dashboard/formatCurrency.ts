const ONE_CENT = 0.01;

/**
 * Locale used when no explicit one is supplied.
 *
 * `Intl` defaults to the **host** locale when it is called with `undefined`,
 * which makes every number in the cost dashboard change shape depending on
 * the operating system's language (a Turkish machine renders `$12,50`, an
 * English one `$12.50`). That is neither the app's active language nor a
 * stable contract, so the default is pinned instead and the dashboard passes
 * the app locale explicitly.
 */
export const DEFAULT_INTL_LOCALE = 'en-US';

function currencyFormatter(
  currency: string,
  maximumFractionDigits: number,
  locale: string
): Intl.NumberFormat {
  const normalized = currency?.trim() ? currency.trim().toUpperCase() : 'USD';
  try {
    return new Intl.NumberFormat(locale, {
      style: 'currency',
      currency: normalized,
      maximumFractionDigits,
    });
  } catch {
    try {
      return new Intl.NumberFormat(locale, {
        style: 'currency',
        currency: 'USD',
        maximumFractionDigits,
      });
    } catch {
      // A malformed locale must not take the whole dashboard down.
      return new Intl.NumberFormat(DEFAULT_INTL_LOCALE, {
        style: 'currency',
        currency: 'USD',
        maximumFractionDigits,
      });
    }
  }
}

interface FormatCurrencyOptions {
  /**
   * Show sub-cent amounts to four decimals instead of collapsing them to
   * "<$0.01". For chart axes, where several ticks below one cent would
   * otherwise all read the same.
   */
  precise?: boolean;
  /**
   * BCP-47 locale for the number format. Defaults to [`DEFAULT_INTL_LOCALE`];
   * pass the app's active locale to localise the separators.
   */
  locale?: string;
}

/**
 * Format a USD-denominated amount with the requested display currency label.
 * Falls back to the locale "USD" formatter when the configured currency
 * code is not a valid ISO-4217 currency Intl supports — this keeps
 * arbitrary display labels (e.g. "USD ($)") from throwing.
 *
 * A non-zero amount below one cent renders as "<$0.01" rather than "$0.00",
 * so real spend never reads as none. Negative zero (an empty Rust sum
 * serialises as `-0.0`) renders as plain zero.
 */
export function formatCurrency(
  amountUsd: number,
  currency: string,
  options: FormatCurrencyOptions = {}
): string {
  const finite = Number.isFinite(amountUsd) ? amountUsd : 0;
  const safe = Object.is(finite, -0) ? 0 : finite;
  const locale = options.locale ?? DEFAULT_INTL_LOCALE;
  if (safe !== 0 && Math.abs(safe) < ONE_CENT) {
    if (options.precise) {
      return currencyFormatter(currency, 4, locale).format(safe);
    }
    return `${safe < 0 ? '-' : ''}<${currencyFormatter(currency, 2, locale).format(ONE_CENT)}`;
  }
  return currencyFormatter(currency, Math.abs(safe) >= 100 ? 0 : 2, locale).format(safe);
}

export function formatTokens(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '0';
  // Round to one decimal in thousands first: 999_960 is "1000.0K" otherwise.
  const thousands = Math.round(n / 100) / 10;
  if (n >= 1_000_000 || thousands >= 1_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${thousands.toFixed(1)}K`;
  return Math.round(n).toString();
}

/** Percentage to one decimal; a non-zero share below 0.1% reads "<0.1%", not "0.0%". */
export function formatPercent(percent: number): string {
  const safe = Number.isFinite(percent) ? Math.max(0, percent) : 0;
  if (safe > 0 && safe < 0.1) return '<0.1%';
  return `${safe.toFixed(1)}%`;
}

export function shortDayLabel(isoDate: string, locale: string = DEFAULT_INTL_LOCALE): string {
  try {
    const date = new Date(`${isoDate}T00:00:00Z`);
    return new Intl.DateTimeFormat(locale, { weekday: 'short', timeZone: 'UTC' }).format(date);
  } catch {
    return isoDate.slice(5);
  }
}

/**
 * Full long-form date label for tooltips (e.g. "Wed, May 27").
 * Falls back to the raw ISO string when parsing fails.
 */
export function longDateLabel(isoDate: string, locale: string = DEFAULT_INTL_LOCALE): string {
  try {
    const date = new Date(`${isoDate}T00:00:00Z`);
    return new Intl.DateTimeFormat(locale, {
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
export function dayOfMonth(isoDate: string, locale: string = DEFAULT_INTL_LOCALE): string {
  try {
    const date = new Date(`${isoDate}T00:00:00Z`);
    return new Intl.DateTimeFormat(locale, { day: 'numeric', timeZone: 'UTC' }).format(date);
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
type RelativeTimeTranslator = (key: string, fallback?: string) => string;

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
