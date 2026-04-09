import { useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useUsageState } from '../../hooks/useUsageState';
import UpsellBanner from './UpsellBanner';
import { dismissBanner, shouldShowBanner } from './upsellDismissState';

const COOLDOWN_WARNING_MS = 24 * 60 * 60 * 1000;
const COOLDOWN_UPGRADE_MS = 7 * 24 * 60 * 60 * 1000;

export default function GlobalUpsellBanner() {
  const navigate = useNavigate();
  const { teamUsage, isLoading, isAtLimit, isNearLimit, isFreeTier, usagePct10h, usagePct7d } =
    useUsageState();

  const [dismissed, setDismissed] = useState<Record<string, boolean>>({});

  if (isLoading || !teamUsage) return null;

  if (isAtLimit) {
    const bannerId = 'global-upgrade';
    if (!shouldShowBanner(bannerId, COOLDOWN_UPGRADE_MS) || dismissed[bannerId]) return null;
    return (
      <div className="fixed top-0 left-0 right-0 z-[9997] px-4 pt-2">
        <UpsellBanner
          variant="upgrade"
          title="Usage limit reached"
          message="Upgrade to continue chatting."
          ctaLabel="Upgrade"
          onCtaClick={() => navigate('/settings/billing')}
          dismissible
          onDismiss={() => {
            dismissBanner(bannerId);
            setDismissed(prev => ({ ...prev, [bannerId]: true }));
          }}
        />
      </div>
    );
  }

  if (isNearLimit && isFreeTier) {
    const bannerId = 'global-warning';
    if (!shouldShowBanner(bannerId, COOLDOWN_WARNING_MS) || dismissed[bannerId]) return null;
    const pct = Math.round(Math.max(usagePct10h, usagePct7d) * 100);
    return (
      <div className="fixed top-0 left-0 right-0 z-[9997] px-4 pt-2">
        <UpsellBanner
          variant="warning"
          title="Approaching usage limit"
          message={`You've used ${pct}% of your usage limit. Upgrade for higher limits.`}
          ctaLabel="Upgrade"
          onCtaClick={() => navigate('/settings/billing')}
          dismissible
          onDismiss={() => {
            dismissBanner(bannerId);
            setDismissed(prev => ({ ...prev, [bannerId]: true }));
          }}
        />
      </div>
    );
  }

  return null;
}
