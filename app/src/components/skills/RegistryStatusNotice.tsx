import { useT } from '../../lib/i18n/I18nContext';
import { DEFAULT_INTL_LOCALE, intlLocale } from '../../lib/i18n/intlLocale';
import type { CatalogPage, ParsedRegistryError } from '../../services/api/skillRegistryApi';
import { Alert, AlertDescription } from '../ui';
import Button from '../ui/Button';

const UNREACHABLE_KINDS = new Set(['timeout', 'unavailable', 'transport']);

/**
 * A registry snapshot timestamp, as a full local date-time.
 *
 * `locale` is a BCP-47 tag — pass `intlLocale(locale)` from `useT()` so the
 * rendering follows the app's active language instead of the host's.
 */
function formatFetchedAt(
  fetchedAt: number | null,
  locale: string = DEFAULT_INTL_LOCALE
): string | null {
  if (fetchedAt == null) return null;
  try {
    return new Date(fetchedAt * 1000).toLocaleString(locale);
  } catch {
    return null;
  }
}

function RetryButton({ onRetry, tone }: { onRetry: () => void; tone?: 'danger' }) {
  const { t } = useT();
  return (
    <Button
      variant="secondary"
      tone={tone}
      size="xs"
      data-testid="registry-retry"
      onClick={onRetry}>
      {t('common.retry')}
    </Button>
  );
}

/** The message a failed catalog read shows when nothing is held to display. */
export function RegistryErrorNotice({
  error,
  onRetry,
}: {
  error: ParsedRegistryError;
  onRetry: () => void;
}) {
  const { t } = useT();
  let text = error.message;
  if (error.kind === 'rate_limited') {
    text =
      error.retryAfterSecs != null
        ? t('skills.registry.rateLimited').replace('{seconds}', String(error.retryAfterSecs))
        : t('skills.registry.rateLimitedShortly');
  } else if (error.kind && UNREACHABLE_KINDS.has(error.kind)) {
    text = t('skills.registry.unreachable');
  }
  return (
    <Alert variant="destructive" density="compact" data-testid="registry-error">
      <AlertDescription className="flex flex-wrap items-center justify-between gap-2">
        <span>{text}</span>
        <RetryButton onRetry={onRetry} tone="danger" />
      </AlertDescription>
    </Alert>
  );
}

/**
 * The state of the catalog behind the page on screen: still loading for the
 * first time, a saved copy shown while a refresh runs, or a saved copy shown
 * because the registry could not be reached.
 */
export function RegistryStatusNotice({
  page,
  firstLoad,
  onRetry,
}: {
  page: CatalogPage | null;
  firstLoad: boolean;
  onRetry: () => void;
}) {
  const { t, locale } = useT();
  if (firstLoad) {
    return (
      <Alert variant="info" density="compact" data-testid="registry-first-load">
        <AlertDescription>{t('skills.registry.firstFetchHint')}</AlertDescription>
      </Alert>
    );
  }
  if (!page) return null;

  const offline = page.lastError != null || page.freshness === 'local_fallback';
  if (offline) {
    const time = formatFetchedAt(page.fetchedAt, intlLocale(locale));
    const text = time
      ? t('skills.registry.offline').replace('{time}', time)
      : t('skills.registry.offlineNoTime');
    return (
      <Alert variant="warning" density="compact" data-testid="registry-offline">
        <AlertDescription className="flex flex-wrap items-center justify-between gap-2">
          <span>{text}</span>
          <RetryButton onRetry={onRetry} />
        </AlertDescription>
      </Alert>
    );
  }
  if (page.refreshing && page.freshness === 'cached') {
    return (
      <Alert variant="info" density="compact" data-testid="registry-refreshing">
        <AlertDescription>{t('skills.registry.refreshing')}</AlertDescription>
      </Alert>
    );
  }
  return null;
}
