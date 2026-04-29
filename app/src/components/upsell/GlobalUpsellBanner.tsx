import { useUsageState } from '../../hooks/useUsageState';
import { BILLING_DASHBOARD_URL } from '../../utils/links';
import { openUrl } from '../../utils/openUrl';
import UpsellBanner from './UpsellBanner';

export default function GlobalUpsellBanner() {
  const { teamUsage, isLoading, isAtLimit, isNearLimit, isFreeTier, usagePct10h, usagePct7d } =
    useUsageState();

  if (isLoading || !teamUsage) return null;

  if (isAtLimit) {
    return (
      <div className="relative z-20">
        <UpsellBanner
          variant="upgrade"
          title="You've reached your usage limit"
          message="Upgrade your plan or top up credits to continue"
          ctaLabel="Upgrade"
          rounded={false}
          onCtaClick={() => {
            void openUrl(BILLING_DASHBOARD_URL);
          }}
        />
      </div>
    );
  }

  if (isNearLimit && isFreeTier) {
    const pct = Math.round(Math.max(usagePct10h, usagePct7d) * 100);
    return (
      <div className="relative z-20">
        <UpsellBanner
          variant="warning"
          title="Approaching usage limit"
          message={`You've used ${pct}% of your usage limit. Upgrade for higher limits.`}
          ctaLabel="Upgrade"
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
