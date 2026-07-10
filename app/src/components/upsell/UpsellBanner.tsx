import { useT } from '../../lib/i18n/I18nContext';

interface UpsellBannerProps {
  variant: 'info' | 'warning' | 'upgrade';
  title: string;
  message: string;
  ctaLabel?: string;
  onCtaClick?: () => void;
  dismissible?: boolean;
  rounded?: boolean;
  onDismiss?: () => void;
}

const VARIANT_STYLES = {
  info: {
    container: 'bg-blue-50 dark:bg-blue-500/15 border-blue-200 dark:border-blue-800',
    icon: 'text-blue-400',
    title: 'text-blue-700 dark:text-blue-300',
    text: 'text-blue-600 dark:text-blue-300',
    cta: 'bg-blue-500 hover:bg-blue-400 text-white',
  },
  warning: {
    container: 'bg-amber-50 dark:bg-amber-500/15 border-amber-200 dark:border-amber-800',
    icon: 'text-amber-400',
    title: 'text-amber-700 dark:text-amber-300',
    text: 'text-amber-600 dark:text-amber-300',
    cta: 'bg-amber-500 hover:bg-amber-400 text-content-inverted',
  },
  upgrade: {
    container: 'bg-amber-50 dark:bg-amber-500/15 border-amber-200 dark:border-amber-800',
    icon: 'text-amber-400',
    title: 'text-amber-700 dark:text-amber-300',
    text: 'text-amber-600 dark:text-amber-300',
    cta: 'bg-amber-500 hover:bg-amber-400 text-content-inverted',
  },
};

export default function UpsellBanner({
  variant,
  title,
  message,
  ctaLabel,
  onCtaClick,
  dismissible,
  onDismiss,
  rounded = true,
}: UpsellBannerProps) {
  const { t } = useT();
  const styles = VARIANT_STYLES[variant];

  return (
    <div
      className={`p-3 ${rounded ? 'rounded-xl' : ''} border flex items-center justify-between gap-3 ${styles.container}`}>
      <div className="flex items-center gap-2 min-w-0">
        <svg
          className={`w-4 h-4 flex-shrink-0 ${styles.icon}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
          />
        </svg>
        <div className="min-w-0">
          <p className={`text-xs font-medium ${styles.title}`}>{title}</p>
          <p className={`text-xs ${styles.text} truncate`}>{message}</p>
        </div>
      </div>
      <div className="flex items-center gap-2 flex-shrink-0">
        {ctaLabel && onCtaClick && (
          <button
            onClick={onCtaClick}
            className={`px-3 py-1.5 rounded-lg text-xs font-medium transition-colors ${styles.cta}`}>
            {ctaLabel}
          </button>
        )}
        {dismissible && onDismiss && (
          <button
            onClick={onDismiss}
            className="p-1 rounded text-content-faint hover:text-content-secondary transition-colors"
            aria-label={t('common.dismiss')}>
            <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M6 18L18 6M6 6l12 12"
              />
            </svg>
          </button>
        )}
      </div>
    </div>
  );
}
