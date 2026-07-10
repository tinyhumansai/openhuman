import { useT } from '../../lib/i18n/I18nContext';
import { BILLING_DASHBOARD_URL, DISCORD_INVITE_URL } from '../../utils/links';
import { openUrl } from '../../utils/openUrl';

function formatUsd(amount: number): string {
  return `$${amount.toFixed(amount % 1 === 0 ? 0 : 2)}`;
}

export function UsageLimitBanner({
  tone,
  icon,
  title,
  message,
  ctaLabel,
  secondaryCtaLabel,
  onSecondaryCtaClick,
}: {
  tone: 'warning' | 'danger';
  icon: string;
  title: string;
  message: string;
  ctaLabel: string;
  secondaryCtaLabel?: string;
  onSecondaryCtaClick?: () => void;
}) {
  const styles =
    tone === 'danger'
      ? {
          card: 'border-coral-200 bg-gradient-to-r from-coral-50 via-rose-50 to-orange-50 dark:border-coral-500/30 dark:from-coral-900/30 dark:via-coral-900/20 dark:to-coral-900/10',
          title: 'text-coral-700 dark:text-coral-300',
          body: 'text-coral-500 dark:text-coral-300/80',
          button:
            'border-coral-700 text-coral-700 hover:text-coral-800 dark:border-coral-300 dark:text-coral-300 dark:hover:text-coral-200',
        }
      : {
          card: 'border-amber-200 bg-gradient-to-r from-amber-50 via-orange-50 to-rose-50 dark:border-amber-500/30 dark:from-amber-900/30 dark:via-amber-900/20 dark:to-amber-900/10',
          title: 'text-amber-700 dark:text-amber-300',
          body: 'text-amber-600 dark:text-amber-300/80',
          button:
            'border-amber-700 text-amber-700 hover:text-amber-800 dark:border-amber-300 dark:text-amber-300 dark:hover:text-amber-200',
        };

  return (
    <div className={`mb-3 rounded-2xl border px-4 py-4 text-left shadow-soft ${styles.card}`}>
      <div className="flex items-start gap-3">
        <div className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-lg`}>
          {icon}
        </div>
        <div className="min-w-0 flex-1">
          <p className={`text-sm font-semibold ${styles.title}`}>{title}</p>
          <p className={`mt-1 text-sm leading-relaxed ${styles.body}`}>
            {message}&nbsp;
            <button
              type="button"
              onClick={() => {
                void openUrl(BILLING_DASHBOARD_URL);
              }}
              className={`cursor-pointer border-b border-dashed font-bold ${styles.button}`}>
              {ctaLabel}
            </button>
            {secondaryCtaLabel && onSecondaryCtaClick && (
              <>
                {' '}
                <button
                  type="button"
                  onClick={onSecondaryCtaClick}
                  className={`cursor-pointer border-b border-dashed font-bold ${styles.button}`}>
                  {secondaryCtaLabel}
                </button>
              </>
            )}
          </p>
        </div>
      </div>
    </div>
  );
}

export function PromotionalCreditsBanner({ promoCredits }: { promoCredits: number }) {
  const { t } = useT();
  return (
    <div className="mb-3 rounded-2xl border border-amber-200 bg-gradient-to-r from-amber-50 via-orange-50 to-rose-50 px-4 py-2.5 text-left shadow-soft dark:border-amber-500/30 dark:from-amber-900/30 dark:via-amber-900/20 dark:to-amber-900/10">
      <div className="flex items-start gap-2.5">
        <div className="mt-px flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-amber-100 text-base dark:bg-amber-500/20">
          🎉
        </div>
        <p className="min-w-0 flex-1 text-sm leading-relaxed text-amber-600 dark:text-amber-300/80">
          {(() => {
            // Single {amount} template; split so the amount renders bold inline.
            const [before, after] = t('home.banners.promoCreditsBody').split('{amount}');
            return (
              <>
                {before}
                <span className="font-semibold text-amber-700 dark:text-amber-300">
                  {formatUsd(promoCredits)}
                </span>
                {after}
              </>
            );
          })()}{' '}
          <button
            type="button"
            onClick={() => {
              void openUrl(BILLING_DASHBOARD_URL);
            }}
            className="cursor-pointer border-b border-dashed border-amber-700 font-bold text-amber-700 hover:text-amber-800 dark:border-amber-300 dark:text-amber-300 dark:hover:text-amber-200">
            {t('home.banners.getSubscription')}
          </button>{' '}
          {t('home.banners.promoCreditsUsage')}
        </p>
      </div>
    </div>
  );
}

export function EarlyBirdyBanner({ onDismiss }: { onDismiss?: () => void }) {
  const { t } = useT();
  return (
    <div className="relative mb-3 mt-3 rounded-2xl border border-orange-200 bg-gradient-to-r from-orange-50 via-amber-50 to-orange-50 px-4 py-4 text-left shadow-soft dark:border-orange-500/30 dark:from-orange-900/30 dark:via-amber-900/20 dark:to-orange-900/10">
      {onDismiss && (
        <button
          type="button"
          onClick={onDismiss}
          aria-label={t('home.banners.earlyBirdDismiss')}
          className="absolute right-3 top-3 rounded-md p-1 text-orange-500 hover:bg-orange-100 hover:text-orange-700 dark:text-orange-300 dark:hover:bg-orange-500/10 dark:hover:text-orange-200">
          ✕
        </button>
      )}
      <div className="flex items-start gap-3">
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-orange-100 dark:bg-orange-500/20 text-lg">
          🐦
        </div>
        <div className="min-w-0 flex-1">
          <p className="text-sm font-semibold text-orange-700 dark:text-orange-300">
            {t('home.banners.earlyBirdTitle')}
          </p>
          <p className="mt-1 text-sm leading-relaxed text-orange-600 dark:text-orange-300/80">
            {t('home.banners.earlyBirdUseCode')}{' '}
            <span className="rounded-md border border-orange-300 bg-surface px-1.5 py-0.5 font-mono text-[12px] font-bold text-orange-700 dark:border-orange-500/40 dark:text-orange-300">
              EARLYBIRDY
            </span>{' '}
            {t('home.banners.earlyBirdOn')}{' '}
            <button
              type="button"
              onClick={() => {
                void openUrl(BILLING_DASHBOARD_URL);
              }}
              className="cursor-pointer border-b border-amber-700 border-dashed font-bold text-amber-700 hover:text-amber-800 dark:border-amber-300 dark:text-amber-300 dark:hover:text-amber-200">
              {t('home.banners.earlyBirdFirstSub')}
            </button>{' '}
          </p>
        </div>
      </div>
    </div>
  );
}

export function DiscordBanner() {
  const { t } = useT();
  return (
    <button
      type="button"
      onClick={() => {
        void openUrl(DISCORD_INVITE_URL);
      }}
      className="mb-3 text-left mt-3 block w-full rounded-2xl border border-[#CDD2FF] bg-gradient-to-r from-[#F6F7FF] via-[#F1F3FF] to-[#ECEFFF] px-4 py-2.5 text-[#414AAE] shadow-soft transition-transform transition-colors hover:-translate-y-0.5 hover:border-[#BCC3FF] hover:from-[#EEF0FF] hover:to-[#E5E9FF] dark:border-[#5865F2]/30 dark:from-[#5865F2]/10 dark:via-[#5865F2]/15 dark:to-[#5865F2]/10 dark:text-[#A5B0FF] dark:hover:border-[#5865F2]/50 dark:hover:from-[#5865F2]/15 dark:hover:to-[#5865F2]/20">
      <div className="flex items-center gap-2.5">
        <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-[#5865F2]/12 text-[#5865F2]">
          <svg className="h-4 w-4" fill="currentColor" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M20.317 4.37A19.79 19.79 0 0 0 15.885 3c-.191.328-.403.775-.552 1.124a18.27 18.27 0 0 0-5.29 0A11.56 11.56 0 0 0 9.49 3a19.74 19.74 0 0 0-4.433 1.37C2.253 8.51 1.492 12.55 1.872 16.533a19.9 19.9 0 0 0 5.239 2.673c.423-.58.8-1.196 1.123-1.845a12.84 12.84 0 0 1-1.767-.85c.148-.106.292-.217.43-.332c3.408 1.6 7.104 1.6 10.472 0c.14.115.283.226.43.332c-.565.338-1.157.623-1.771.851c.322.648.698 1.264 1.123 1.844a19.84 19.84 0 0 0 5.241-2.673c.446-4.617-.761-8.621-3.787-12.164ZM9.46 14.088c-1.02 0-1.855-.936-1.855-2.084c0-1.148.82-2.084 1.855-2.084c1.044 0 1.87.944 1.855 2.084c0 1.148-.82 2.084-1.855 2.084Zm5.08 0c-1.02 0-1.855-.936-1.855-2.084c0-1.148.82-2.084 1.855-2.084c1.044 0 1.87.944 1.855 2.084c0 1.148-.812 2.084-1.855 2.084Z" />
          </svg>
        </div>
        <div className="min-w-0 flex-1 text-sm">
          <span className="font-semibold">{t('home.banners.discordTitle')}</span>{' '}
          <span className="text-[#5E66BC] dark:text-[#8B95DD]">
            {t('home.banners.discordSubtitle')}
          </span>
        </div>
      </div>
    </button>
  );
}
