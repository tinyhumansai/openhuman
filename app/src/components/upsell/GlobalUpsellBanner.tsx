import { useUsageState } from '../../hooks/useUsageState';
import { useT } from '../../lib/i18n/I18nContext';
import { BILLING_DASHBOARD_URL } from '../../utils/links';
import { openUrl } from '../../utils/openUrl';
import UpsellBanner from './UpsellBanner';

export default function GlobalUpsellBanner() {
  const { t } = useT();
  const { teamUsage, isLoading, isAtLimit, isNearLimit, isFreeTier, usagePct } = useUsageState();

  if (isLoading || !teamUsage) return null;

  if (isAtLimit) {
    return (
      <div className="relative z-20">
        <UpsellBanner
          variant="upgrade"
          title={t('upsell.global.limitTitle')}
          message={t('upsell.global.limitMessage')}
          ctaLabel={t('chat.upgrade')}
          rounded={false}
          onCtaClick={() => {
            void openUrl(BILLING_DASHBOARD_URL);
          }}
        />
      </div>
    );
  }

  if (isNearLimit && isFreeTier) {
    const pct = Math.round(usagePct * 100);
    return (
      <div className="relative z-20">
        <UpsellBanner
          variant="warning"
          title={t('upsell.global.nearLimitTitle')}
          message={t('upsell.global.nearLimitMessage').replace('{pct}', String(pct))}
          ctaLabel={t('chat.upgrade')}
          rounded={false}
          onCtaClick={() => {
            void openUrl(BILLING_DASHBOARD_URL);
          }}
        />
      </div>
    );
  }

  return null;
}
