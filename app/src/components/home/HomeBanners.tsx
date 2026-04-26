import { DISCORD_INVITE_URL } from '../../utils/links';
import { openUrl } from '../../utils/openUrl';

const BILLING_DASHBOARD_URL = 'https://tinyhumans.ai/dashboard';

function formatUsd(amount: number): string {
  return `$${amount.toFixed(amount % 1 === 0 ? 0 : 2)}`;
}

export function UsageLimitBanner({
  tone,
  icon,
  title,
  message,
  ctaLabel,
}: {
  tone: 'warning' | 'danger';
  icon: string;
  title: string;
  message: string;
  ctaLabel: string;
}) {
  const styles =
    tone === 'danger'
      ? {
          card: 'border-coral-200 bg-gradient-to-r from-coral-50 via-rose-50 to-orange-50',
          title: 'text-coral-700',
          body: 'text-coral-500',
          button: 'border-coral-700 text-coral-700 hover:text-coral-800',
        }
      : {
          card: 'border-amber-200 bg-gradient-to-r from-amber-50 via-orange-50 to-rose-50',
          title: 'text-amber-700',
          body: 'text-amber-600',
          button: 'border-amber-700 text-amber-700 hover:text-amber-800',
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
          </p>
        </div>
      </div>
    </div>
  );
}

export function PromotionalCreditsBanner({ promoCredits }: { promoCredits: number }) {
  return (
    <div className="mb-3 rounded-2xl border border-amber-200 bg-gradient-to-r from-amber-50 via-orange-50 to-rose-50 px-4 py-4 text-left shadow-soft">
      <div className="flex items-start gap-3">
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-amber-100 text-lg">
          🎉
        </div>
        <div className="min-w-0 flex-1">
          <p className="text-sm font-semibold text-amber-700">
            You have {formatUsd(promoCredits)} of promotional credits.
          </p>
          <p className="mt-1 text-sm leading-relaxed text-amber-600">
            Give OpenHuman a spin, and when you&apos;re ready for more,{' '}
            <button
              type="button"
              onClick={() => {
                void openUrl(BILLING_DASHBOARD_URL);
              }}
              className="cursor-pointer border-b border-amber-700 border-dashed font-bold text-amber-700 hover:text-amber-800">
              get a subscription
            </button>{' '}
            and get 10x more usage.
          </p>
        </div>
      </div>
    </div>
  );
}

export function DiscordBanner() {
  return (
    <button
      type="button"
      onClick={() => {
        void openUrl(DISCORD_INVITE_URL);
      }}
      className="mb-3 text-left mt-3 block rounded-2xl border border-[#CDD2FF] bg-gradient-to-r from-[#F6F7FF] via-[#F1F3FF] to-[#ECEFFF] px-4 py-4 text-[#414AAE] shadow-soft transition-transform transition-colors hover:-translate-y-0.5 hover:border-[#BCC3FF] hover:from-[#EEF0FF] hover:to-[#E5E9FF]">
      <div className="flex items-center gap-3">
        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-[#5865F2]/12 text-[#5865F2]">
          <svg className="h-5 w-5" fill="currentColor" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M20.317 4.37A19.79 19.79 0 0 0 15.885 3c-.191.328-.403.775-.552 1.124a18.27 18.27 0 0 0-5.29 0A11.56 11.56 0 0 0 9.49 3a19.74 19.74 0 0 0-4.433 1.37C2.253 8.51 1.492 12.55 1.872 16.533a19.9 19.9 0 0 0 5.239 2.673c.423-.58.8-1.196 1.123-1.845a12.84 12.84 0 0 1-1.767-.85c.148-.106.292-.217.43-.332c3.408 1.6 7.104 1.6 10.472 0c.14.115.283.226.43.332c-.565.338-1.157.623-1.771.851c.322.648.698 1.264 1.123 1.844a19.84 19.84 0 0 0 5.241-2.673c.446-4.617-.761-8.621-3.787-12.164ZM9.46 14.088c-1.02 0-1.855-.936-1.855-2.084c0-1.148.82-2.084 1.855-2.084c1.044 0 1.87.944 1.855 2.084c0 1.148-.82 2.084-1.855 2.084Zm5.08 0c-1.02 0-1.855-.936-1.855-2.084c0-1.148.82-2.084 1.855-2.084c1.044 0 1.87.944 1.855 2.084c0 1.148-.812 2.084-1.855 2.084Z" />
          </svg>
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-semibold">Join Our Discord</div>
          <div className="mt-0.5 text-sm text-[#5E66BC]">
            Get updates, free merch, credits, report bugs, and be part of the OpenHuman community.
          </div>
        </div>
      </div>
    </button>
  );
}
